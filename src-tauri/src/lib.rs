//! Mods Manager: shared lists of exact mod files, one per game, and a Sync button that makes
//! this computer match the list.

mod config;
mod db;
mod install;
mod sources;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine as _;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_opener::OpenerExt;
use tokio::sync::{Mutex, RwLock};
use tokio_postgres::Client;

use config::{GamePaths, Installed, Local, LocalGame, Root, Settings};
use db::{Event, Game, Member, NewGame, NewMod, SharedMod};
use sources::{nexus, AddOptions, Ctx, Resolved, SearchHit};
use tauri_plugin_deep_link::DeepLinkExt;

type R<T> = Result<T, String>;

pub struct AppState {
    data_dir: PathBuf,
    temp: PathBuf,
    settings: RwLock<Settings>,
    local: Mutex<Local>,
    db: Mutex<Option<Arc<Client>>>,
    http: reqwest::Client,
    /// Everything that installs or removes files, in order.
    queue: Queue,
    listener: std::sync::Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    /// Live updates are connected (also sent as the `live` event).
    live: AtomicBool,
    /// v0.1.0's local data was looked at this run.
    local_migrated: AtomicBool,
    /// One-time Nexus download keys from clicked `nxm://` links.
    nxm: Arc<nexus::NxmKeys>,
    /// Who the Nexus API key belongs to (checked once per key).
    nexus_account: Mutex<Option<nexus::Account>>,
}

impl AppState {
    fn settings_path(&self) -> PathBuf {
        self.data_dir.join("settings.json")
    }
    fn local_path(&self) -> PathBuf {
        self.data_dir.join("local.json")
    }

    async fn db(&self) -> R<Arc<Client>> {
        let client = {
            let mut slot = self.db.lock().await;
            match slot.as_ref() {
                Some(c) if !c.is_closed() => c.clone(),
                _ => {
                    let url = self.settings.read().await.database_url.clone();
                    if url.trim().is_empty() {
                        return Err("Set up the shared database in Settings first".into());
                    }
                    let c = Arc::new(db::connect(&url).await?);
                    *slot = Some(c.clone());
                    c
                }
            }
        };
        if !self.local_migrated.swap(true, Ordering::SeqCst) {
            if let Err(e) = self.migrate_local(&client).await {
                self.local_migrated.store(false, Ordering::SeqCst);
                return Err(format!("Moving your BONELAB setup to the new version failed: {e}"));
            }
        }
        Ok(client)
    }

    /// v0.1.0 had one game (BONELAB): its folders and installed mods become the BONELAB game's,
    /// so nothing downloads again.
    async fn migrate_local(&self, client: &Client) -> R<()> {
        let mut settings = self.settings.write().await;
        let mut local = self.local.lock().await;
        let old_folders = !settings.mods_dir.is_empty() || !settings.game_dir.is_empty();
        let old_installs = !local.installed.is_empty() || !local.ignored.is_empty();
        if !old_folders && !old_installs {
            return Ok(());
        }
        let id = db::bonelab(client).await?;
        if old_folders {
            let (content, game) = (std::mem::take(&mut settings.mods_dir), std::mem::take(&mut settings.game_dir));
            let paths = settings.games.entry(id).or_default();
            if paths.content_dir.is_empty() {
                paths.content_dir = content;
            }
            if paths.game_dir.is_empty() {
                paths.game_dir = game;
            }
            settings.current_game.get_or_insert(id);
        }
        if old_installs {
            let installed = std::mem::take(&mut local.installed);
            let ignored = std::mem::take(&mut local.ignored);
            let game = local.game_mut(id);
            game.installed.extend(installed);
            game.ignored.extend(ignored.into_iter().map(|k| match k.strip_prefix("mods/") {
                Some(rest) => format!("content/{rest}"),
                None => k,
            }));
        }
        config::save(&self.local_path(), &*local)?;
        config::save(&self.settings_path(), &*settings)
    }

    async fn save_local(&self, local: &Local) -> R<()> {
        config::save(&self.local_path(), local)
    }

    async fn me(&self) -> String {
        self.settings.read().await.name.trim().to_string()
    }

    /// Everything a command about one game needs.
    async fn game(&self, id: i64) -> R<(Arc<Client>, Settings, Game, GamePaths)> {
        let db = self.db().await?;
        let game = db::game(&db, id).await?;
        let settings = self.settings.read().await.clone();
        let paths = settings.paths(id);
        Ok((db, settings, game, paths))
    }

    fn ctx<'a>(&'a self, settings: &'a Settings, game: &'a Game) -> Ctx<'a> {
        Ctx { http: &self.http, settings, temp: &self.temp, game, nxm: &self.nxm }
    }

    /// The Nexus account behind the API key (None without a key or when Nexus can't be reached).
    async fn nexus_account(&self, refresh: bool) -> Option<nexus::Account> {
        let key = self.settings.read().await.nexus_api_key.trim().to_string();
        if key.is_empty() {
            return None;
        }
        let mut cached = self.nexus_account.lock().await;
        if cached.is_none() || refresh {
            if let Ok(a) = nexus::account(&self.http, &key).await {
                *cached = Some(a);
            }
        }
        cached.clone()
    }
}

// ── the plan: what this computer has to do to match a game's list ─────────

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ModView {
    #[serde(flatten)]
    m: SharedMod,
    /// installed | missing | outdated | broken
    state: &'static str,
    local_version: Option<String>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PlanItem {
    key: String,
    id: Option<i64>,
    name: String,
    /// install | update | repair | remove
    action: &'static str,
    from: Option<String>,
    to: Option<String>,
    size: i64,
    target: String,
    /// Nexus Mods without Premium: the page to click "Slow download" on
    manual_url: Option<String>,
}

fn state_of(paths: &GamePaths, local: &LocalGame, m: &SharedMod) -> &'static str {
    match local.installed.get(&m.data.key()) {
        None => "missing",
        Some(i) if i.pin != m.data.pin() => "outdated",
        Some(i) if !install::is_present(paths, i) => "broken",
        Some(_) => "installed",
    }
}

fn plan(paths: &GamePaths, local: &LocalGame, mods: &[SharedMod]) -> Vec<PlanItem> {
    let mut out = Vec::new();
    for m in mods {
        let inst = local.installed.get(&m.data.key());
        let action = match state_of(paths, local, m) {
            "missing" => "install",
            "outdated" => "update",
            "broken" => "repair",
            _ => continue,
        };
        out.push(PlanItem {
            key: m.data.key(),
            id: Some(m.id),
            name: m.data.name.clone(),
            action,
            from: inst.map(|i| i.version.clone()),
            to: Some(m.data.version.clone()),
            size: m.data.file_size,
            target: m.data.target.clone(),
            manual_url: None,
        });
    }
    for (key, inst) in &local.installed {
        if !mods.iter().any(|m| &m.data.key() == key) {
            out.push(PlanItem {
                key: key.clone(),
                id: None,
                name: inst.name.clone(),
                action: "remove",
                from: Some(inst.version.clone()),
                to: None,
                size: 0,
                target: if inst.root == Root::Game { "code".into() } else { "content".into() },
                manual_url: None,
            });
        }
    }
    out
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    me: String,
    game: Game,
    paths: GamePaths,
    mods: Vec<ModView>,
    plan: Vec<PlanItem>,
    members: Vec<Member>,
    unmanaged: usize,
    /// The game's mod loader is in its folder (true without a loader)
    loader_ok: bool,
    /// Code mods are on the list, so the loader is needed
    needs_loader: bool,
    live: bool,
    /// The Nexus account, when a key is set
    nexus: Option<nexus::Account>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameView {
    #[serde(flatten)]
    game: Game,
    mods: i64,
    paths: GamePaths,
}

// ── commands: settings ─────────────────────────────────────────────────────

#[tauri::command]
async fn get_settings(state: State<'_, AppState>) -> R<Settings> {
    Ok(state.settings.read().await.clone())
}

#[tauri::command]
async fn save_settings(app: AppHandle, state: State<'_, AppState>, mut settings: Settings) -> R<()> {
    let (url_changed, nexus_changed) = {
        let old = state.settings.read().await;
        (old.database_url != settings.database_url, old.nexus_api_key != settings.nexus_api_key || old.nexus_ignore_links != settings.nexus_ignore_links)
    };
    {
        let mut saved = state.settings.write().await;
        settings.nexus_logged_in = saved.nexus_logged_in;
        config::save(&state.settings_path(), &settings)?;
        *saved = settings;
    }
    if url_changed {
        *state.db.lock().await = None;
        start_listener(&app);
    }
    if nexus_changed {
        *state.nexus_account.lock().await = None;
        apply_nxm(&app).await;
    }
    Ok(())
}

/// The Nexus account behind the key: name, and whether it's Premium.
#[tauri::command]
async fn nexus_account(state: State<'_, AppState>) -> R<Option<nexus::Account>> {
    let key = state.settings.read().await.nexus_api_key.trim().to_string();
    if key.is_empty() {
        return Ok(None);
    }
    let a = nexus::account(&state.http, &key).await?;
    *state.nexus_account.lock().await = Some(a.clone());
    Ok(Some(a))
}

/// A "Mod Manager Download" from the website (also pasted by hand): remember its one-time key.
#[tauri::command]
fn nexus_link(app: AppHandle, url: String) -> R<()> {
    handle_nxm(&app, &url)
}

/// Mods Manager opens `nxm://` links while a Nexus key is set (unless left to e.g. Vortex).
async fn apply_nxm(app: &AppHandle) {
    let s = app.state::<AppState>().settings.read().await.clone();
    let dl = app.deep_link();
    if !s.nexus_api_key.trim().is_empty() && !s.nexus_ignore_links {
        let _ = dl.register("nxm");
    } else if dl.is_registered("nxm").unwrap_or(false) {
        let _ = dl.unregister("nxm");
    }
}

fn handle_nxm(app: &AppHandle, link: &str) -> R<()> {
    let l = nexus::parse_nxm(link).ok_or("That isn't a Nexus Mods download link (nxm://…)")?;
    let key = l.key.ok_or("That link has no download key; use \"Mod Manager Download\" on the file's page")?;
    let state = app.state::<AppState>();
    let slot = format!("{}:{}", l.mod_id, l.file_id);
    state.nxm.lock().unwrap().insert(slot.clone(), key);
    let woke = {
        let mut jobs = state.queue.jobs.lock().unwrap();
        let mut woke = false;
        for j in jobs.iter_mut().filter(|j| j.status == "waiting" && j.nexus.as_deref() == Some(&slot)) {
            j.status = "queued";
            j.error = None;
            woke = true;
        }
        woke
    };
    if woke {
        let _ = app.emit("queue", state.queue.snapshot());
        kick(app);
    }
    let _ = app.emit("nexus-link", serde_json::json!({ "domain": l.domain, "modId": l.mod_id, "fileId": l.file_id }));
    // clicked in the in-app Nexus window: stay there, it moves on to the next file
    if app.get_webview_window(NEXUS_WINDOW).is_some() {
        return Ok(());
    }
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
    Ok(())
}

#[tauri::command]
fn detect_folders(steam_dir: String, content_hint: String) -> GamePaths {
    config::detect(&steam_dir, &content_hint)
}

#[tauri::command]
async fn test_connection(url: String) -> R<()> {
    db::connect(&url).await.map(|_| ())
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Invite {
    database_url: String,
    game: Option<i64>,
}

const INVITE_PREFIX: &str = "MM1-";

/// A code that sets up someone else's app: the database (with its password) and a game.
#[tauri::command]
async fn invite_code(state: State<'_, AppState>, game: Option<i64>) -> R<String> {
    let url = state.settings.read().await.database_url.clone();
    if url.trim().is_empty() {
        return Err("Set up the shared database first".into());
    }
    let json = serde_json::to_vec(&Invite { database_url: url, game }).map_err(|e| e.to_string())?;
    Ok(format!("{INVITE_PREFIX}{}", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(json)))
}

#[tauri::command]
fn read_invite(code: String) -> R<Invite> {
    let body = code.trim().strip_prefix(INVITE_PREFIX).ok_or("That isn't an invite code (they start with MM1-)")?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(body).map_err(|_| "That invite code is damaged; copy it again")?;
    serde_json::from_slice(&bytes).map_err(|_| "That invite code is damaged; copy it again".to_string())
}

// ── commands: games ────────────────────────────────────────────────────────

#[tauri::command]
fn presets() -> Vec<NewGame> {
    db::presets()
}

#[tauri::command]
async fn list_games(state: State<'_, AppState>) -> R<Vec<GameView>> {
    let db = state.db().await?;
    let settings = state.settings.read().await.clone();
    let counts = db::mod_counts(&db).await?;
    Ok(db::games(&db)
        .await?
        .into_iter()
        .map(|g| GameView { mods: counts.get(&g.id).copied().unwrap_or(0), paths: settings.paths(g.id), game: g })
        .collect())
}

/// Cleans up what was typed and fills in the mod.io game id from its slug (mod.io/g/<slug>).
async fn with_modio_id(state: &AppState, mut game: NewGame) -> R<NewGame> {
    game.name = game.name.trim().to_string();
    if game.name.is_empty() {
        return Err("Give the game a name".into());
    }
    if !["melonloader", "bepinex", "none"].contains(&game.loader.as_str()) {
        game.loader = "none".into();
    }
    // a pasted community link: thunderstore.io/c/<community>/…
    let ts = game.thunderstore.trim().trim_end_matches('/');
    game.thunderstore = ts.rsplit("/c/").next().unwrap_or("").split('/').next().unwrap_or("").to_string();
    game.nexus = nexus::game_domain(&game.nexus);
    let slug = game.modio_slug.trim().to_string();
    if slug.is_empty() {
        game.modio_id = 0;
        game.modio_slug.clear();
    } else if game.modio_id <= 0 || slug.contains('/') {
        let settings = state.settings.read().await.clone();
        let (id, found) = sources::modio::game_id(&state.http, &settings, &slug)
            .await
            .map_err(|e| format!("Looking up the game on mod.io: {e}"))?;
        game.modio_id = id;
        if !found.is_empty() {
            game.modio_slug = found;
        }
    }
    Ok(game)
}

#[tauri::command]
async fn create_game(state: State<'_, AppState>, game: NewGame, paths: GamePaths) -> R<i64> {
    let game = with_modio_id(&state, game).await?;
    let db = state.db().await?;
    let me = state.me().await;
    let id = db::insert_game(&db, &game, &me).await?;
    db::notify(&db, id, &me, "game", &game.name).await?;
    let mut settings = state.settings.write().await;
    settings.games.insert(id, paths);
    settings.current_game = Some(id);
    config::save(&state.settings_path(), &*settings)?;
    Ok(id)
}

#[tauri::command]
async fn update_game(state: State<'_, AppState>, id: i64, game: NewGame, paths: GamePaths) -> R<()> {
    let game = with_modio_id(&state, game).await?;
    let db = state.db().await?;
    db::update_game(&db, id, &game).await?;
    let me = state.me().await;
    db::notify(&db, id, &me, "game", &game.name).await?;
    let mut settings = state.settings.write().await;
    settings.games.insert(id, paths);
    config::save(&state.settings_path(), &*settings)
}

/// Saves where a game lives on this computer (nothing shared changes).
#[tauri::command]
async fn set_game_paths(state: State<'_, AppState>, id: i64, paths: GamePaths) -> R<()> {
    let mut settings = state.settings.write().await;
    settings.games.insert(id, paths);
    config::save(&state.settings_path(), &*settings)
}

/// Removes the game and its list for everyone. Installed files stay where they are.
#[tauri::command]
async fn delete_game(state: State<'_, AppState>, id: i64) -> R<()> {
    let db = state.db().await?;
    let game = db::game(&db, id).await?;
    db::delete_game(&db, id).await?;
    let me = state.me().await;
    db::notify(&db, id, &me, "game-removed", &game.data.name).await?;
    let mut settings = state.settings.write().await;
    settings.games.remove(&id);
    if settings.current_game == Some(id) {
        settings.current_game = None;
    }
    config::save(&state.settings_path(), &*settings)?;
    let mut local = state.local.lock().await;
    local.games.remove(&id);
    state.save_local(&local).await
}

#[tauri::command]
async fn set_current_game(state: State<'_, AppState>, id: Option<i64>) -> R<()> {
    let mut settings = state.settings.write().await;
    settings.current_game = id;
    config::save(&state.settings_path(), &*settings)
}

// ── commands: a game's list ────────────────────────────────────────────────

#[tauri::command]
async fn overview(state: State<'_, AppState>, game: i64) -> R<Overview> {
    let (db, settings, g, paths) = state.game(game).await?;
    let mods = db::list_mods(&db, game).await?;
    let local = state.local.lock().await.game(game);
    let mut plan = plan(&paths, &local, &mods);
    let has_nexus = mods.iter().any(|m| m.data.source == "nexus");
    let nexus_acc = if has_nexus || !settings.nexus_api_key.trim().is_empty() { state.nexus_account(false).await } else { None };
    if has_nexus && !nexus_acc.as_ref().is_some_and(|a| a.premium) {
        for item in plan.iter_mut() {
            if let Some(m) = mods.iter().find(|m| Some(m.id) == item.id && m.data.source == "nexus") {
                item.manual_url = Some(nexus::file_page(&nexus::game_domain(&g.data.nexus), &m.data.source_id, &m.data.file_ref));
            }
        }
    }
    let me = settings.name.trim().to_string();
    if !me.is_empty() {
        db::heartbeat(&db, &me, game, plan.len() as i32, plan.is_empty()).await?;
    }
    let members = db::members(&db, game).await?;
    let views = mods
        .into_iter()
        .map(|m| {
            let st = state_of(&paths, &local, &m);
            let local_version = local.installed.get(&m.data.key()).map(|i| i.version.clone());
            ModView { m, state: st, local_version }
        })
        .collect::<Vec<_>>();
    let needs_loader = g.data.loader != "none" && views.iter().any(|v| v.m.data.target == "code");
    Ok(Overview {
        me,
        unmanaged: install::unmanaged(&paths, &g.data.loader, &local).len(),
        loader_ok: install::loader_installed(&paths, &g.data.loader),
        needs_loader,
        live: state.live.load(Ordering::Relaxed),
        nexus: nexus_acc,
        game: g,
        paths,
        mods: views,
        plan,
        members,
    })
}

#[tauri::command]
async fn resolve_mod(state: State<'_, AppState>, game: i64, input: String, name: Option<String>, target: Option<String>) -> R<Resolved> {
    let (_, settings, g, _) = state.game(game).await?;
    let opts = AddOptions { name: name.unwrap_or_default(), target: target.unwrap_or_else(|| "content".into()) };
    sources::resolve(&state.ctx(&settings, &g), &input, &opts).await
}

#[tauri::command]
async fn search_modio(state: State<'_, AppState>, game: i64, query: String) -> R<Vec<SearchHit>> {
    let (_, settings, g, _) = state.game(game).await?;
    sources::modio::search(&state.ctx(&settings, &g), &query).await
}

/// Puts mods on a game's list; returns how many were new.
#[tauri::command]
async fn add_mods(state: State<'_, AppState>, game: i64, mods: Vec<NewMod>) -> R<usize> {
    let db = state.db().await?;
    let me = state.me().await;
    let mut added = 0;
    for m in &mods {
        if db::insert_mod(&db, game, m, &me).await? {
            added += 1;
            db::record(&db, game, &me, "add", &m.name, &m.version).await?;
        }
    }
    Ok(added)
}

/// Takes a mod off the list for everyone; each computer deletes its files on its next sync.
#[tauri::command]
async fn remove_mod(state: State<'_, AppState>, id: i64) -> R<()> {
    let db = state.db().await?;
    let me = state.me().await;
    let m = db::get_mod(&db, id).await?;
    db::delete_mod(&db, id).await?;
    db::record(&db, m.game_id, &me, "remove", &m.data.name, &m.data.version).await
}

/// Newer files for the mods on a list (links aren't checked: that would download them all).
#[tauri::command]
async fn check_updates(state: State<'_, AppState>, game: i64) -> R<HashMap<i64, NewMod>> {
    let (db, settings, g, _) = state.game(game).await?;
    let mods = db::list_mods(&db, game).await?;
    // each lookup owns what it needs: borrowed futures in a stream trip up the command's Send bound
    let (settings, g) = (Arc::new(settings), Arc::new(g));
    let lookups = mods.into_iter().map(|m| {
        let (http, settings, temp, g, nxm) = (state.http.clone(), settings.clone(), state.temp.clone(), g.clone(), state.nxm.clone());
        tauri::async_runtime::spawn(async move {
            let ctx = Ctx { http: &http, settings: &settings, temp: &temp, game: &g, nxm: &nxm };
            (m.id, sources::latest(&ctx, &m.data, false).await)
        })
    });
    let found: Vec<(i64, R<Option<NewMod>>)> = futures_util::stream::iter(lookups)
        .buffer_unordered(6)
        .filter_map(|r| async move { r.ok() })
        .collect()
        .await;
    let mut out = HashMap::new();
    let mut first_err = None;
    for (id, r) in found {
        match r {
            Ok(Some(n)) => {
                out.insert(id, n);
            }
            Ok(None) => {}
            Err(e) => first_err = first_err.or(Some(e)),
        }
    }
    if out.is_empty() {
        if let Some(e) = first_err {
            return Err(e);
        }
    }
    Ok(out)
}

/// Re-pins a mod to its newest file, for everyone.
#[tauri::command]
async fn update_mod(state: State<'_, AppState>, id: i64) -> R<Option<String>> {
    let db = state.db().await?;
    let cur = db::get_mod(&db, id).await?;
    let (_, settings, g, _) = state.game(cur.game_id).await?;
    let me = settings.name.trim().to_string();
    let Some(next) = sources::latest(&state.ctx(&settings, &g), &cur.data, true).await? else { return Ok(None) };
    db::repin(&db, id, &next, &me).await?;
    db::record(&db, cur.game_id, &me, "update", &next.name, &format!("{} -> {}", cur.data.version, next.version)).await?;
    Ok(Some(next.version))
}

#[tauri::command]
async fn events(state: State<'_, AppState>, game: i64) -> R<Vec<Event>> {
    let db = state.db().await?;
    db::events(&db, game, 300).await
}

// ── the install queue ──────────────────────────────────────────────────────
//
// Everything that changes files goes through one queue: Sync, "install right away" after adding,
// auto-sync and Nexus clicks just add jobs. Up to `PARALLEL` run at once; each job reports its
// own progress. A mod is never queued twice, and the window sees the whole queue on every change
// (the `queue` event).

/// Jobs that run at the same time.
const PARALLEL: usize = 3;
/// Finished jobs kept for the panel.
const KEEP_FINISHED: usize = 50;

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    id: u64,
    game: i64,
    game_name: String,
    /// `source:source_id`, as in the local state
    key: String,
    /// The mod on the list (None for removals)
    mod_id: Option<i64>,
    name: String,
    /// install | update | repair | remove
    action: &'static str,
    /// queued | downloading | installing | removing | done | failed | cancelled | waiting
    status: &'static str,
    done: u64,
    total: u64,
    error: Option<String>,
    /// Nexus without Premium: the page to click "Slow download" on
    manual_url: Option<String>,
    /// Nexus `mod_id:file_id`, to match a clicked nxm:// link
    nexus: Option<String>,
}

impl Job {
    fn active(&self) -> bool {
        matches!(self.status, "queued" | "downloading" | "installing" | "removing")
    }
    fn finished(&self) -> bool {
        matches!(self.status, "done" | "failed" | "cancelled")
    }
}

#[derive(Default)]
pub struct Queue {
    jobs: std::sync::Mutex<Vec<Job>>,
    next_id: std::sync::atomic::AtomicU64,
    /// A worker is draining the queue.
    working: AtomicBool,
    /// Running downloads asked to stop.
    cancel: std::sync::Mutex<std::collections::HashSet<u64>>,
}

/// How the queue tells the window (or a test) that it changed.
type QueueEmit = dyn Fn(Vec<Job>) + Send + Sync;

impl Queue {
    fn snapshot(&self) -> Vec<Job> {
        self.jobs.lock().unwrap().clone()
    }
    fn update(&self, id: u64, f: impl FnOnce(&mut Job)) {
        if let Some(j) = self.jobs.lock().unwrap().iter_mut().find(|j| j.id == id) {
            f(j);
        }
    }
    fn get(&self, id: u64) -> Option<Job> {
        self.jobs.lock().unwrap().iter().find(|j| j.id == id).cloned()
    }
    fn reorder(&self, order: &[u64]) {
        let mut jobs = self.jobs.lock().unwrap();
        let slots: Vec<usize> = jobs.iter().enumerate().filter(|(_, j)| j.status == "queued").map(|(i, _)| i).collect();
        let mut queued: Vec<Job> = slots.iter().map(|&i| jobs[i].clone()).collect();
        let rank = |j: &Job| order.iter().position(|id| *id == j.id).unwrap_or(usize::MAX);
        queued.sort_by_key(rank); // stable: unlisted jobs keep their order, after the listed ones
        for (slot, job) in slots.into_iter().zip(queued) {
            jobs[slot] = job;
        }
    }

    fn is_cancelled(&self, id: u64) -> bool {
        self.cancel.lock().unwrap().contains(&id)
    }

    /// Stops a job: one that hasn't started (or waits for its Nexus click) leaves the queue,
    /// a download stops at its next chunk. Installing and removing finish (seconds), so a mod is
    /// never left half-installed.
    fn cancel(&self, id: u64) -> R<()> {
        let mut jobs = self.jobs.lock().unwrap();
        let Some(j) = jobs.iter().find(|j| j.id == id) else { return Ok(()) };
        match j.status {
            "queued" | "waiting" => jobs.retain(|j| j.id != id),
            "downloading" => {
                self.cancel.lock().unwrap().insert(id);
            }
            "installing" | "removing" => return Err(format!("{} is almost done and can't be stopped now", j.name)),
            _ => {}
        }
        Ok(())
    }
}

/// The folder a code mod gets for itself (BepInEx plugins): Thunderstore's `Namespace-Name`.
fn package_name(m: &NewMod) -> String {
    if m.source == "thunderstore" {
        m.source_id.replace('/', "-")
    } else {
        install::sanitize(&m.name)
    }
}

/// Downloads and installs one mod; `progress(phase, done, total)` reports along the way.
#[allow(clippy::too_many_arguments)]
async fn install_one(
    progress: &(dyn Fn(&'static str, u64, u64) + Send + Sync),
    cancelled: &(dyn Fn() -> bool + Send + Sync),
    state: &AppState,
    settings: &Settings,
    game: &Game,
    paths: &GamePaths,
    m: &SharedMod,
    old: Option<Installed>,
) -> R<Installed> {
    let code = m.data.target == "code" && game.data.loader != "none";
    if code && paths.game_dir.trim().is_empty() {
        return Err(format!("Set the {} game folder first (Edit game): code mods go there", game.data.name));
    }
    if !code && paths.content_dir.trim().is_empty() {
        return Err(format!("Set the {} mods folder first (Edit game)", game.data.name));
    }
    let ctx = state.ctx(settings, game);
    let fetch = sources::fetch(&ctx, &m.data).await?;
    let file = state.temp.join(format!("dl-{}", m.id));
    let staging = state.temp.join(format!("unpack-{}", m.id));
    let mut last = Instant::now() - Duration::from_secs(1);
    let got = install::download(
        fetch,
        &file,
        |done, total| {
            if last.elapsed() >= Duration::from_millis(150) || done == total {
                last = Instant::now();
                progress("downloading", done, total);
            }
        },
        cancelled,
    )
    .await;
    let got = match got {
        Ok(g) => g,
        Err(e) => {
            let _ = std::fs::remove_file(&file);
            return Err(e);
        }
    };
    progress("installing", 0, 0);
    let (paths2, loader, package, name) = (paths.clone(), game.data.loader.clone(), package_name(&m.data), m.data.name.clone());
    let (file2, staging2) = (file.clone(), staging.clone());
    let placed = tauri::async_runtime::spawn_blocking(move || -> R<(Root, Vec<String>)> {
        install::unpack(&file2, &got.file_name, &staging2)?;
        // take the old version out first, so renamed folders don't stay behind
        if let Some(old) = &old {
            install::remove(&paths2, old)?;
        }
        if code {
            Ok((Root::Game, install::place_code(&staging2, Path::new(&paths2.game_dir), &loader, &package)?))
        } else {
            Ok((Root::Content, install::place_content(&staging2, Path::new(&paths2.content_dir), &name)?))
        }
    })
    .await
    .map_err(|e| e.to_string())
    .and_then(|r| r);
    let _ = std::fs::remove_file(&file);
    let _ = std::fs::remove_dir_all(&staging);
    let (root, placed) = placed?;
    Ok(Installed { name: m.data.name.clone(), pin: m.data.pin(), version: m.data.version.clone(), root, paths: placed, installed_at: chrono::Utc::now() })
}

/// Adds what this computer has to do for a game to the queue; returns how many jobs were added.
/// Free Nexus files go in as `waiting` until their click arrives.
async fn enqueue(state: &AppState, game: i64) -> R<usize> {
    let (db, settings, g, paths) = state.game(game).await?;
    let mods = db::list_mods(&db, game).await?;
    let items = plan(&paths, &state.local.lock().await.game(game), &mods);
    let premium = if mods.iter().any(|m| m.data.source == "nexus") { state.nexus_account(false).await.is_some_and(|a| a.premium) } else { false };
    let logged_in = settings.nexus_logged_in;
    let mut jobs = state.queue.jobs.lock().unwrap();
    let mut added = 0;
    for item in items {
        // already queued, running or waiting for its Nexus click
        if jobs.iter().any(|j| j.game == game && j.key == item.key && (j.active() || j.status == "waiting")) {
            continue;
        }
        let m = mods.iter().find(|m| Some(m.id) == item.id);
        let nexus = m.filter(|m| m.data.source == "nexus").map(|m| format!("{}:{}", m.data.source_id, m.data.file_ref));
        let has_key = nexus.as_ref().is_some_and(|k| state.nxm.lock().unwrap().contains_key(k));
        let manual = item.action != "remove" && nexus.is_some() && (!logged_in || (!premium && !has_key));
        jobs.push(Job {
            id: state.queue.next_id.fetch_add(1, Ordering::SeqCst) + 1,
            game,
            game_name: g.data.name.clone(),
            key: item.key.clone(),
            mod_id: item.id,
            name: item.name.clone(),
            action: item.action,
            status: if manual { "waiting" } else { "queued" },
            done: 0,
            total: item.size.max(0) as u64,
            error: None,
            manual_url: if manual { m.map(|m| nexus::file_page(&nexus::game_domain(&g.data.nexus), &m.data.source_id, &m.data.file_ref)) } else { None },
            nexus,
        });
        added += 1;
    }
    // keep the panel short: drop the oldest finished jobs
    let finished = jobs.iter().filter(|j| j.finished()).count();
    if finished > KEEP_FINISHED {
        let mut drop = finished - KEEP_FINISHED;
        jobs.retain(|j| {
            if drop > 0 && j.finished() {
                drop -= 1;
                false
            } else {
                true
            }
        });
    }
    Ok(added)
}

/// Runs one job to its end.
async fn run_job(state: &AppState, id: u64, emit: &QueueEmit) {
    run_job_inner(state, id, emit).await;
    state.queue.cancel.lock().unwrap().remove(&id);
}

async fn run_job_inner(state: &AppState, id: u64, emit: &QueueEmit) {
    let Some(job) = state.queue.get(id) else { return };
    let report = |f: &dyn Fn(&mut Job)| {
        state.queue.update(id, |j| f(j));
        emit(state.queue.snapshot());
    };
    let outcome: R<Option<Option<Installed>>> = async {
        let (db, settings, g, paths) = state.game(job.game).await?;
        let old = state.local.lock().await.game(job.game).installed.get(&job.key).cloned();
        if job.action == "remove" {
            report(&|j| j.status = "removing");
            let Some(old) = old else { return Ok(Some(None)) };
            let p = paths.clone();
            tauri::async_runtime::spawn_blocking(move || install::remove(&p, &old)).await.map_err(|e| e.to_string())??;
            return Ok(Some(None));
        }
        // the list may have changed since it was queued: install what's on it now
        let m = match job.mod_id {
            Some(mid) => db::get_mod(&db, mid).await.ok(),
            None => None,
        };
        let Some(m) = m else { return Ok(None) };
        if old.as_ref().is_some_and(|o| o.pin == m.data.pin() && install::is_present(&paths, o)) {
            return Ok(None);
        }
        let progress = |phase: &'static str, done: u64, total: u64| {
            report(&|j| {
                j.status = phase;
                j.done = done;
                if total > 0 {
                    j.total = total;
                }
            })
        };
        let cancelled = || state.queue.is_cancelled(id);
        install_one(&progress, &cancelled, state, &settings, &g, &paths, &m, old).await.map(|i| Some(Some(i)))
    }
    .await;
    match outcome {
        // nothing left to do (already matched, or the mod left the list)
        Ok(None) => report(&|j| j.status = "done"),
        Ok(Some(installed)) => {
            let mut local = state.local.lock().await;
            let lg = local.game_mut(job.game);
            match installed {
                Some(i) => {
                    lg.installed.insert(job.key.clone(), i);
                }
                None => {
                    lg.installed.remove(&job.key);
                }
            }
            let saved = state.save_local(&local).await;
            drop(local);
            report(&|j| match &saved {
                Ok(()) => {
                    j.status = "done";
                    j.done = j.total;
                }
                Err(e) => {
                    j.status = "failed";
                    j.error = Some(e.clone());
                }
            });
        }
        Err(e) if e == install::CANCELLED => report(&|j| {
            j.status = "cancelled";
            j.done = 0;
        }),
        Err(e) => match e.strip_prefix(nexus::NEEDS_CLICK) {
            Some(page) => {
                let page = page.to_string();
                report(&|j| {
                    j.status = "waiting";
                    j.manual_url = Some(page.clone());
                    j.done = 0;
                })
            }
            None => report(&|j| {
                j.status = "failed";
                j.error = Some(e.clone());
            }),
        },
    }
}

/// Takes the next queued job (the queue's order is the download order) and marks it started.
fn start_next(queue: &Queue, touched: &mut Vec<i64>) -> Option<u64> {
    let mut jobs = queue.jobs.lock().unwrap();
    let j = jobs.iter_mut().find(|j| j.status == "queued")?;
    j.status = if j.action == "remove" { "removing" } else { "downloading" };
    if !touched.contains(&j.game) {
        touched.push(j.game);
    }
    Some(j.id)
}

/// Works through the queue until nothing is queued, up to `PARALLEL` jobs at a time (a slot that
/// frees up takes the next job right away); then tells the others where this computer stands in
/// each game it touched.
async fn drain(state: &AppState, emit: &QueueEmit) {
    let mut touched: Vec<i64> = Vec::new();
    let mut running = futures_util::stream::FuturesUnordered::new();
    loop {
        while running.len() < PARALLEL {
            let Some(id) = start_next(&state.queue, &mut touched) else { break };
            emit(state.queue.snapshot());
            running.push(run_job(state, id, emit));
        }
        if running.next().await.is_none() {
            break;
        }
    }
    let me = state.me().await;
    if me.is_empty() {
        return;
    }
    for game in touched {
        let Ok((db, _, _, paths)) = state.game(game).await else { continue };
        let Ok(mods) = db::list_mods(&db, game).await else { continue };
        let left = plan(&paths, &state.local.lock().await.game(game), &mods).len() as i32;
        let _ = db::heartbeat(&db, &me, game, left, left == 0).await;
        let _ = db::notify(&db, game, &me, "synced", "").await;
    }
}

/// Starts a worker unless one is running.
fn kick(app: &AppHandle) {
    let state = app.state::<AppState>();
    if state.queue.working.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        let emitter = app.clone();
        let emit = move |jobs: Vec<Job>| {
            let _ = emitter.emit("queue", jobs);
        };
        loop {
            drain(&state, &emit).await;
            state.queue.working.store(false, Ordering::SeqCst);
            // something was queued between the last check and letting go: go again
            let more = state.queue.jobs.lock().unwrap().iter().any(|j| j.status == "queued");
            if !more || state.queue.working.swap(true, Ordering::SeqCst) {
                break;
            }
        }
        emit(state.queue.snapshot());
    });
}

/// Queues everything this computer has to do for a game.
#[tauri::command]
async fn queue_sync(app: AppHandle, state: State<'_, AppState>, game: i64) -> R<usize> {
    let added = enqueue(&state, game).await?;
    let _ = app.emit("queue", state.queue.snapshot());
    kick(&app);
    Ok(added)
}

#[tauri::command]
fn queue_list(state: State<'_, AppState>) -> Vec<Job> {
    state.queue.snapshot()
}

/// Stops a job (see `Queue::cancel`).
#[tauri::command]
fn queue_cancel(app: AppHandle, state: State<'_, AppState>, id: u64) -> R<()> {
    let r = state.queue.cancel(id);
    let _ = app.emit("queue", state.queue.snapshot());
    r
}

/// Stops everything that can be stopped; returns how many.
#[tauri::command]
fn queue_cancel_all(app: AppHandle, state: State<'_, AppState>) -> usize {
    let ids: Vec<u64> = state.queue.snapshot().into_iter().filter(|j| matches!(j.status, "queued" | "waiting" | "downloading")).map(|j| j.id).collect();
    let n = ids.iter().filter(|id| state.queue.cancel(**id).is_ok()).count();
    let _ = app.emit("queue", state.queue.snapshot());
    n
}

/// Puts the queued jobs in this order (the order they download in). Jobs that already run or
/// finished keep their place; queued jobs missing from `order` go after the ones listed.
#[tauri::command]
fn queue_reorder(app: AppHandle, state: State<'_, AppState>, order: Vec<u64>) {
    state.queue.reorder(&order);
    let _ = app.emit("queue", state.queue.snapshot());
}

#[tauri::command]
fn queue_retry(app: AppHandle, state: State<'_, AppState>, id: u64) {
    state.queue.update(id, |j| {
        if j.status == "failed" || j.status == "cancelled" {
            j.status = "queued";
            j.error = None;
            j.done = 0;
        }
    });
    let _ = app.emit("queue", state.queue.snapshot());
    kick(&app);
}

#[tauri::command]
fn queue_clear(app: AppHandle, state: State<'_, AppState>) {
    state.queue.jobs.lock().unwrap().retain(|j| !j.finished());
    let _ = app.emit("queue", state.queue.snapshot());
}

// ── commands: housekeeping ─────────────────────────────────────────────────

#[tauri::command]
async fn unmanaged(state: State<'_, AppState>, game: i64) -> R<Vec<install::Unmanaged>> {
    let (_, _, g, paths) = state.game(game).await?;
    let local = state.local.lock().await.game(game);
    Ok(install::unmanaged(&paths, &g.data.loader, &local))
}

#[tauri::command]
async fn delete_unmanaged(state: State<'_, AppState>, game: i64, keys: Vec<String>) -> R<()> {
    let paths = state.settings.read().await.paths(game);
    for key in keys {
        install::delete_unmanaged(&paths, &key)?;
    }
    Ok(())
}

/// Unmanaged items (`content/<folder>`, `game/<path>`) as one install: its root and paths.
fn adopted_paths(keys: &[String]) -> R<(Root, Vec<String>)> {
    let mut root = None;
    let mut paths = Vec::new();
    for key in keys {
        let (r, rel) = if let Some(rel) = key.strip_prefix("content/") {
            (Root::Content, rel)
        } else if let Some(rel) = key.strip_prefix("game/") {
            (Root::Game, rel)
        } else {
            return Err(format!("Unknown file {key}"));
        };
        if *root.get_or_insert(r) != r {
            return Err("A mod's files are either in the mods folder or in the game folder, not both".into());
        }
        if install::safe_join(std::path::Path::new("x"), rel).is_none() {
            return Err(format!("Unknown file {key}"));
        }
        paths.push(rel.to_string());
    }
    Ok((root.ok_or("Choose at least one file or folder")?, paths))
}

/// "These files are that mod": records files already in the game's folders as the install of a
/// mod on the list, so it isn't downloaded again. `exact`: they're the pinned version (counts as
/// installed); otherwise it counts as another version and the next sync replaces them.
#[tauri::command]
async fn adopt(state: State<'_, AppState>, game: i64, mod_id: i64, keys: Vec<String>, exact: bool) -> R<()> {
    let (db, _, _, paths) = state.game(game).await?;
    let m = db::get_mod(&db, mod_id).await?;
    if m.game_id != game {
        return Err("That mod belongs to another game".into());
    }
    let (root, rels) = adopted_paths(&keys)?;
    let base = install::root_dir(&paths, root);
    if base.as_os_str().is_empty() {
        return Err("Set this game's folders first (Edit game)".into());
    }
    for rel in &rels {
        if !install::safe_join(&base, rel).is_some_and(|p| p.exists()) {
            return Err(format!("{rel} isn't there anymore"));
        }
    }
    let mut local = state.local.lock().await;
    let lg = local.game_mut(game);
    // the files may have been flagged "keep" before; they belong to a mod now
    for key in &keys {
        lg.ignored.remove(key);
    }
    lg.installed.insert(
        m.data.key(),
        Installed {
            name: m.data.name.clone(),
            pin: if exact { m.data.pin() } else { "adopted|".into() },
            version: if exact { m.data.version.clone() } else { "unknown".into() },
            root,
            paths: rels,
            installed_at: chrono::Utc::now(),
        },
    );
    state.save_local(&local).await
}

#[tauri::command]
async fn ignore_unmanaged(state: State<'_, AppState>, game: i64, keys: Vec<String>, ignore: bool) -> R<()> {
    let mut local = state.local.lock().await;
    let lg = local.game_mut(game);
    for key in keys {
        if ignore {
            lg.ignored.insert(key);
        } else {
            lg.ignored.remove(&key);
        }
    }
    state.save_local(&local).await
}

/// Copies the list from the old Mod.io-Mods-Manager tables onto a game (pinned to today's files).
#[tauri::command]
async fn import_legacy(state: State<'_, AppState>, game: i64) -> R<(usize, Vec<String>)> {
    let (db, settings, g, _) = state.game(game).await?;
    let me = settings.name.trim().to_string();
    let legacy = db::legacy(&db).await?.ok_or("This database has no tables from the old Mod.io Mods Manager")?;
    let ctx = state.ctx(&settings, &g);
    let (mut added, mut failed) = (0, Vec::new());
    let mut found: Vec<R<NewMod>> = Vec::new();
    for (id, name) in &legacy.modio {
        found.push(sources::modio::resolve(&ctx, &id.to_string()).await.map_err(|e| format!("{name}: {e}")));
    }
    for (ns, name, ver) in &legacy.code {
        let ver = Some(ver.as_str()).filter(|v| !v.is_empty());
        found.push(sources::thunderstore::pinned(&ctx, ns, name, ver).await.map(|(m, _)| m).map_err(|e| format!("{name}: {e}")));
    }
    for m in found {
        match m {
            Ok(m) => {
                if db::insert_mod(&db, game, &m, &me).await? {
                    added += 1;
                }
            }
            Err(e) => failed.push(e),
        }
    }
    if added > 0 {
        db::record(&db, game, &me, "import", &format!("{added} mods"), "from the old manager").await?;
    }
    Ok((added, failed))
}

#[tauri::command]
async fn open_folder(app: AppHandle, state: State<'_, AppState>, game: i64, which: String) -> R<()> {
    let paths = state.settings.read().await.paths(game);
    let path = match which.as_str() {
        "content" => paths.content_dir,
        "game" => paths.game_dir,
        _ => return Err("Unknown folder".into()),
    };
    if path.is_empty() {
        return Err("That folder isn't set yet (Edit game)".into());
    }
    app.opener().open_path(path, None::<&str>).map_err(|e| e.to_string())
}

#[tauri::command]
fn open_url(app: AppHandle, url: String) -> R<()> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("Not a web link".into());
    }
    app.opener().open_url(url, None::<&str>).map_err(|e| e.to_string())
}

const NEXUS_WINDOW: &str = "nexus";
static NEXUS_LOGIN_PENDING: AtomicBool = AtomicBool::new(false);
const NEXUS_SIGN_IN: &str = "https://users.nexusmods.com/auth/sign_in?redirect_url=https%3A%2F%2Fwww.nexusmods.com%2F";

async fn set_nexus_login(app: &AppHandle, logged_in: bool) -> R<()> {
    let state = app.state::<AppState>();
    let mut settings = state.settings.write().await;
    if settings.nexus_logged_in == logged_in { return Ok(()); }
    settings.nexus_logged_in = logged_in;
    config::save(&state.settings_path(), &*settings)?;
    let _ = app.emit("nexus-login", logged_in);
    Ok(())
}

#[tauri::command]
async fn nexus_login(app: AppHandle) -> R<()> {
    set_nexus_login(&app, false).await?;
    let target = app.state::<AppState>().queue.jobs.lock().unwrap().iter()
        .find(|j| j.status == "waiting" && j.manual_url.is_some())
        .and_then(|j| j.manual_url.clone())
        .unwrap_or_else(|| "https://www.nexusmods.com/".into());
    nexus_browse(app, target, None, Some(true)).await
}
static NEXUS_TARGET: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

// A document-title message does not navigate away from a login form or interrupt a POST.
async fn nexus_browser_signal(app: AppHandle, action: &str) -> R<()> {
    let Some(w) = app.get_webview_window(NEXUS_WINDOW) else { return Ok(()); };
    match action {
        "show" => { let _ = w.show(); let _ = w.set_focus(); }
        "signed-out" => {
            set_nexus_login(&app, false).await?;
            let _ = w.show();
            if w.url().ok().is_some_and(|u| u.host_str() != Some("users.nexusmods.com")) {
                NEXUS_LOGIN_PENDING.store(true, Ordering::Relaxed);
                w.navigate(NEXUS_SIGN_IN.parse().unwrap()).map_err(|e| e.to_string())?;
            }
        }
        "signed-in" => {
            set_nexus_login(&app, true).await?;
            NEXUS_LOGIN_PENDING.store(false, Ordering::Relaxed);
            let target = NEXUS_TARGET.lock().unwrap().clone();
            if let Some(target) = target {
                if target == "https://www.nexusmods.com/" { let _ = w.hide(); }
                else if w.url().ok().is_some_and(|u| matches!(u.path(), "/" | "/games" | "/games/")) {
                    w.navigate(target.parse().map_err(|_| "Invalid Nexus file URL")?).map_err(|e| e.to_string())?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

/// Loads a Nexus file in a hidden persistent browser, revealed for login or recovery. The
/// helper selects "Slow download"; its `nxm://` navigation is caught here instead of
/// going through the browser and Windows. The window gets no access to the app's commands.
/// Async on purpose: a window made from a synchronous command on Windows hangs, blank and white.
#[tauri::command]
async fn nexus_browse(app: AppHandle, url: String, name: Option<String>, show: Option<bool>) -> R<()> {
    let mut parsed: url::Url = url.parse().map_err(|_| "Not a web link")?;
    if parsed.scheme() != "https" || !parsed.host_str().is_some_and(|h| h == "nexusmods.com" || h.ends_with(".nexusmods.com")) {
        return Err("Not a Nexus Mods page".into());
    }
    let title = match name {
        Some(n) if !n.trim().is_empty() => format!("Nexus Mods: {}", n.trim()),
        _ => "Nexus Mods".to_string(),
    };
    let needs_login = !app.state::<AppState>().settings.read().await.nexus_logged_in;
    if needs_login {
        NEXUS_LOGIN_PENDING.store(true, Ordering::Relaxed);
        parsed = NEXUS_SIGN_IN.parse().unwrap();
    }
    let previous = NEXUS_TARGET.lock().unwrap().replace(url.clone());
    if let Some(w) = app.get_webview_window(NEXUS_WINDOW) {
        if show.unwrap_or(false) && !needs_login && previous.as_deref() == Some(url.as_str()) {
            let _ = w.show();
            let _ = w.set_focus();
            return Ok(());
        }
        if needs_login || show.unwrap_or(false) { let _ = w.show(); let _ = w.set_focus(); } else { let _ = w.hide(); }
        w.navigate(parsed).map_err(|e| e.to_string())?;
        let _ = w.set_title(&title);
        return Ok(());
    }
    let handle = app.clone();
    let title_handle = app.clone();
    let w = tauri::WebviewWindowBuilder::new(&app, NEXUS_WINDOW, tauri::WebviewUrl::External(parsed))
        .title(title)
        .visible(needs_login || show.unwrap_or(false))
        .on_document_title_changed(move |w, title| {
            if !w.url().ok().is_some_and(|u| u.scheme() == "https" && matches!(u.host_str(), Some("www.nexusmods.com" | "nexusmods.com"))) { return; }
            let Some(message) = title.strip_prefix("__mm_nexus__:") else { return; };
            let action = message.split(':').next().unwrap_or("").to_string();
            if !matches!(action.as_str(), "show" | "signed-in" | "signed-out") { return; }
            let app = title_handle.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = nexus_browser_signal(app.clone(), &action).await {
                    let _ = app.emit("nexus-error", e);
                }
            });
        })
        .on_page_load(|w, payload| {
            if payload.event() != tauri::webview::PageLoadEvent::Finished { return; }
            if !payload.url().host_str().is_some_and(|h| h == "nexusmods.com" || h.ends_with(".nexusmods.com")) { return; }
            if payload.url().host_str() == Some("users.nexusmods.com") {
                // Leave login, two-factor authentication, and verification pages untouched.
                let _ = w.show();
                return;
            }
            if let Some(target) = NEXUS_TARGET.lock().unwrap().as_ref() {
                let script = include_str!("nexus_helper.js").replace("__MM_INFO__", &serde_json::json!({
                    "url": target,
                    "loginReturn": NEXUS_LOGIN_PENDING.load(Ordering::Relaxed)
                }).to_string());
                let _ = w.eval(&script);
            }
        })
        .inner_size(1100.0, 820.0)
        .min_inner_size(600.0, 500.0)
        .on_navigation(move |u| {
            if u.host_str() == Some("users.nexusmods.com") && u.path().starts_with("/auth/sign_in") {
                NEXUS_LOGIN_PENDING.store(true, Ordering::Relaxed);
            }
            if u.scheme() == "nxm" {
                if let Some(w) = handle.get_webview_window(NEXUS_WINDOW) { let _ = w.hide(); }
                let (app, link) = (handle.clone(), u.to_string());
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = handle_nxm(&app, &link) {
                        let _ = app.emit("nexus-error", e);
                    }
                });
                return false;
            }
            u.scheme() == "https" && u.host_str().is_some_and(|h| h == "nexusmods.com" || h.ends_with(".nexusmods.com"))
        })
        .build()
        .map_err(|e| e.to_string())?;
    let handle = app.clone();
    w.on_window_event(move |e| {
        if matches!(e, tauri::WindowEvent::Destroyed) {
            let _ = handle.emit("nexus-closed", ());
        }
    });
    Ok(())
}

#[tauri::command]
async fn nexus_browse_close(app: AppHandle) {
    if let Some(w) = app.get_webview_window(NEXUS_WINDOW) {
        let _ = w.close();
    }
}

#[tauri::command]
fn app_version(app: AppHandle) -> String {
    app.package_info().version.to_string()
}

// ── live updates ───────────────────────────────────────────────────────────

/// Listens for changes to any list and forwards them to the window as `remote-changed`.
fn start_listener(app: &AppHandle) {
    let state = app.state::<AppState>();
    let mut slot = state.listener.lock().unwrap();
    if let Some(old) = slot.take() {
        old.abort();
    }
    let app2 = app.clone();
    *slot = Some(tauri::async_runtime::spawn(async move {
        loop {
            let url = app2.state::<AppState>().settings.read().await.database_url.clone();
            if url.trim().is_empty() {
                return;
            }
            app2.state::<AppState>().live.store(true, Ordering::Relaxed);
            let _ = app2.emit("live", true);
            let app3 = app2.clone();
            let _ = db::listen(&url, move |payload| {
                let _ = app3.emit("remote-changed", payload);
            })
            .await;
            app2.state::<AppState>().live.store(false, Ordering::Relaxed);
            let _ = app2.emit("live", false);
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    }));
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // first: a second start (an nxm:// link clicked while open) hands its link to this one
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            // an nxm:// link clicked in the browser starts a second copy; its link lands here
            for arg in argv.iter().filter(|a| a.starts_with("nxm://")) {
                let _ = handle_nxm(app, arg);
            }
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let temp = app.path().app_cache_dir()?.join("work");
            std::fs::create_dir_all(&data_dir)?;
            let _ = std::fs::remove_dir_all(&temp);
            std::fs::create_dir_all(&temp)?;
            let settings: Settings = config::load(&data_dir.join("settings.json"));
            let local: Local = config::load(&data_dir.join("local.json"));
            let http = reqwest::Client::builder()
                .user_agent(concat!("ModsManager/", env!("CARGO_PKG_VERSION")))
                .connect_timeout(Duration::from_secs(15))
                .build()?;
            app.manage(AppState {
                data_dir,
                temp,
                settings: RwLock::new(settings),
                local: Mutex::new(local),
                db: Mutex::new(None),
                http,
                queue: Queue::default(),
                listener: std::sync::Mutex::new(None),
                live: AtomicBool::new(false),
                local_migrated: AtomicBool::new(false),
                nxm: Arc::new(Default::default()),
                nexus_account: Mutex::new(None),
            });
            start_listener(app.handle());
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move { apply_nxm(&handle).await });
            // started by a click on an nxm:// link: keep its key, tell the window once it's listening
            // (the deep-link plugin only reports schemes the installer registers, and the installer
            // shouldn't take nxm:// from Vortex, so the arguments are read here)
            let links: Vec<String> = std::env::args().skip(1).filter(|a| a.starts_with("nxm://")).collect();
            if !links.is_empty() {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(3)).await;
                    for l in links {
                        let _ = handle_nxm(&handle, &l);
                    }
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            nexus_account,
            nexus_link,
            detect_folders,
            test_connection,
            invite_code,
            read_invite,
            presets,
            list_games,
            create_game,
            update_game,
            set_game_paths,
            delete_game,
            set_current_game,
            overview,
            resolve_mod,
            search_modio,
            add_mods,
            remove_mod,
            check_updates,
            update_mod,
            events,
            queue_sync,
            queue_list,
            queue_cancel,
            queue_cancel_all,
            queue_retry,
            queue_reorder,
            queue_clear,
            unmanaged,
            delete_unmanaged,
            ignore_unmanaged,
            adopt,
            import_legacy,
            open_folder,
            open_url,
            nexus_browse,
            nexus_browse_close,
            nexus_login,
            app_version,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Mods Manager");
}

/// Real database, real downloads. Needs a throwaway Postgres (it drops the mm_* tables):
/// `MM_TEST_DB=postgres://... cargo test -- --ignored --test-threads=1`
#[cfg(test)]
mod e2e {
    use super::*;

    /// One computer: its own folders and state, sharing the database.
    fn computer(name: &str, url: &str, settings: impl FnOnce(&Path) -> Settings) -> AppState {
        let base = std::env::temp_dir().join(format!("mm-e2e-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        for d in ["data", "temp", "content", "game/Mods", "game/MelonLoader"] {
            std::fs::create_dir_all(base.join(d)).unwrap();
        }
        let mut s = settings(&base);
        s.name = name.into();
        s.database_url = url.into();
        AppState {
            data_dir: base.join("data"),
            temp: base.join("temp"),
            settings: RwLock::new(s),
            local: Mutex::new(Local::default()),
            db: Mutex::new(None),
            http: reqwest::Client::builder().user_agent("ModsManager-test").build().unwrap(),
            queue: Queue::default(),
            listener: std::sync::Mutex::new(None),
            live: AtomicBool::new(false),
            local_migrated: AtomicBool::new(false),
            nxm: Arc::new(Default::default()),
            nexus_account: Mutex::new(None),
        }
    }

    fn folders(base: &Path) -> GamePaths {
        GamePaths { content_dir: base.join("content").to_string_lossy().into(), game_dir: base.join("game").to_string_lossy().into() }
    }

    /// Syncs a game; returns what the plan said to do.
    async fn sync(state: &AppState, game: i64) -> Vec<&'static str> {
        let db = state.db().await.unwrap();
        let paths = state.settings.read().await.paths(game);
        let mods = db::list_mods(&db, game).await.unwrap();
        let actions = plan(&paths, &state.local.lock().await.game(game), &mods).iter().map(|p| p.action).collect();
        enqueue(state, game).await.unwrap();
        // queueing twice adds nothing
        assert_eq!(enqueue(state, game).await.unwrap(), 0);
        drain(state, &|_| {}).await;
        let jobs = state.queue.snapshot();
        let failed: Vec<_> = jobs.iter().filter(|j| j.status != "done").map(|j| (&j.name, j.status, &j.error)).collect();
        assert!(failed.is_empty(), "{failed:?}");
        state.queue.jobs.lock().unwrap().clear();
        actions
    }

    async fn raw(url: &str) -> Client {
        let (c, conn) = tokio_postgres::connect(url, tokio_postgres::NoTls).await.unwrap();
        tokio::spawn(conn);
        c
    }

    async fn reset(url: &str) {
        raw(url).await.batch_execute("DROP TABLE IF EXISTS mm_mods, mm_events, mm_members, mm_games, mm_presence").await.unwrap();
    }

    #[tokio::test]
    #[ignore]
    async fn migrates_from_v010() {
        let url = std::env::var("MM_TEST_DB").expect("MM_TEST_DB");
        reset(&url).await;
        // a database exactly as v0.1.0 left it: no games, one mod, one event
        raw(&url)
            .await
            .batch_execute(
                "CREATE TABLE mm_mods (id BIGSERIAL PRIMARY KEY, source TEXT NOT NULL, source_id TEXT NOT NULL, target TEXT NOT NULL DEFAULT 'content',
                   name TEXT NOT NULL, version TEXT NOT NULL DEFAULT '', file_ref TEXT NOT NULL DEFAULT '', file_hash TEXT NOT NULL DEFAULT '',
                   file_size BIGINT NOT NULL DEFAULT 0, page_url TEXT NOT NULL DEFAULT '', thumb_url TEXT NOT NULL DEFAULT '', summary TEXT NOT NULL DEFAULT '',
                   author TEXT NOT NULL DEFAULT '', extra JSONB NOT NULL DEFAULT '{}', added_by TEXT NOT NULL, added_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                   updated_by TEXT NOT NULL, updated_at TIMESTAMPTZ NOT NULL DEFAULT now(), UNIQUE (source, source_id));
                 CREATE TABLE mm_events (id BIGSERIAL PRIMARY KEY, at TIMESTAMPTZ NOT NULL DEFAULT now(), actor TEXT NOT NULL, action TEXT NOT NULL,
                   mod_name TEXT NOT NULL, detail TEXT NOT NULL DEFAULT '');
                 CREATE TABLE mm_members (name TEXT PRIMARY KEY, last_seen TIMESTAMPTZ NOT NULL DEFAULT now(), pending INT NOT NULL DEFAULT 0, synced_at TIMESTAMPTZ);
                 INSERT INTO mm_mods (source, source_id, target, name, version, file_ref, added_by, updated_by)
                   VALUES ('thunderstore', 'Lakatrazz/Fusion', 'code', 'Fusion', '1.14.3', '1.14.3', 'ramon', 'ramon');
                 INSERT INTO mm_events (actor, action, mod_name, detail) VALUES ('ramon', 'add', 'Fusion', '1.14.3');
                 INSERT INTO mm_members (name) VALUES ('cedric');",
            )
            .await
            .unwrap();

        // ramon's v0.1.0 files, exactly as that version wrote them
        let ramon = computer("ramon-old", &url, |_| Settings::default());
        let base = ramon.data_dir.parent().unwrap().to_path_buf();
        let old = folders(&base);
        std::fs::write(
            ramon.data_dir.join("settings.json"),
            serde_json::json!({ "name": "ramon", "databaseUrl": url, "modioApiKey": "", "modioToken": "", "gameId": 3809,
                                "modsDir": old.content_dir, "gameDir": old.game_dir, "autoSync": false })
            .to_string(),
        )
        .unwrap();
        std::fs::write(
            ramon.data_dir.join("local.json"),
            serde_json::json!({
                "installed": { "thunderstore:Lakatrazz/Fusion": { "name": "Fusion", "pin": "1.14.3|", "version": "1.14.3", "root": "game",
                                                                  "paths": ["Mods/LabFusion.dll"], "installed_at": "2026-11-07T18:00:00Z" } },
                "ignored": ["mods/Old.Pallet"]
            })
            .to_string(),
        )
        .unwrap();
        std::fs::write(base.join("game/Mods/LabFusion.dll"), b"x").unwrap();
        // what the new version loads on start
        *ramon.settings.write().await = config::load(&ramon.data_dir.join("settings.json"));
        *ramon.local.lock().await = config::load(&ramon.data_dir.join("local.json"));

        let db = ramon.db().await.unwrap();
        let games = db::games(&db).await.unwrap();
        assert_eq!(games.len(), 1, "one BONELAB game");
        let bonelab = games[0].id;
        assert_eq!((games[0].data.name.as_str(), games[0].data.modio_id, games[0].data.loader.as_str()), ("BONELAB", db::BONELAB_MODIO, "melonloader"));
        assert_eq!(db::list_mods(&db, bonelab).await.unwrap().len(), 1, "the old mod moved into BONELAB");
        assert_eq!(db::events(&db, bonelab, 10).await.unwrap().len(), 1, "and its activity");
        assert!(db::members(&db, bonelab).await.unwrap().iter().any(|m| m.name == "cedric"), "and its people");

        let s = ramon.settings.read().await.clone();
        assert_eq!(s.current_game, Some(bonelab));
        assert!(s.mods_dir.is_empty() && s.game_dir.is_empty());
        assert_eq!(s.paths(bonelab), old);
        let local = ramon.local.lock().await.game(bonelab);
        assert!(local.installed.contains_key("thunderstore:Lakatrazz/Fusion"));
        assert!(local.ignored.contains("content/Old.Pallet"));
        // written to disk, so the next start doesn't migrate again
        let on_disk: Settings = config::load(&ramon.data_dir.join("settings.json"));
        assert_eq!(on_disk.current_game, Some(bonelab));
        let on_disk: Local = config::load(&ramon.data_dir.join("local.json"));
        assert!(on_disk.installed.is_empty() && on_disk.games.contains_key(&bonelab));

        // Fusion was installed with the same pin: nothing downloads again
        assert!(plan(&s.paths(bonelab), &local, &db::list_mods(&db, bonelab).await.unwrap()).is_empty());

        // an old app that's still running adds a mod without a game: the next connect adopts it
        raw(&url)
            .await
            .batch_execute("INSERT INTO mm_mods (source, source_id, name, added_by, updated_by) VALUES ('modio', '1', 'Late', 'cedric', 'cedric')")
            .await
            .unwrap();
        db::connect(&url).await.unwrap();
        assert_eq!(db::list_mods(&db, bonelab).await.unwrap().len(), 2);
        assert_eq!(db::games(&db).await.unwrap().len(), 1, "no second BONELAB");
    }

    #[tokio::test]
    #[ignore]
    async fn three_friends_two_games() {
        let url = std::env::var("MM_TEST_DB").expect("MM_TEST_DB");
        reset(&url).await;
        let setup = computer("setup", &url, |_| Settings::default());
        let db = setup.db().await.unwrap();
        let presets = db::presets();
        let bonelab = db::insert_game(&db, &presets[0], "ramon").await.unwrap();
        let lethal = db::insert_game(&db, &presets[2], "ramon").await.unwrap();

        let make = |name: &str| {
            computer(name, &url, |base| Settings {
                games: [(bonelab, folders(&base.join("bonelab"))), (lethal, folders(&base.join("lethal")))].into(),
                ..Default::default()
            })
        };
        let (ramon, cedric, alex) = (make("ramon"), make("cedric"), make("alex"));
        for c in [&ramon, &cedric, &alex] {
            let s = c.settings.read().await.clone();
            for p in s.games.values() {
                std::fs::create_dir_all(&p.content_dir).unwrap();
                std::fs::create_dir_all(Path::new(&p.game_dir).join("MelonLoader")).unwrap();
            }
        }

        // everyone hears changes live
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let listen_url = url.clone();
        tokio::spawn(async move {
            let _ = db::listen(&listen_url, move |p| {
                let _ = tx.send(p);
            })
            .await;
        });
        tokio::time::sleep(Duration::from_millis(500)).await;

        // ramon: Fusion (MelonLoader) on BONELAB, pinned to an older version
        let rs = ramon.settings.read().await.clone();
        let bg = db::game(&db, bonelab).await.unwrap();
        let ctx = ramon.ctx(&rs, &bg);
        let (latest, _) = sources::thunderstore::pinned(&ctx, "Lakatrazz", "Fusion", None).await.unwrap();
        let mut older = None;
        for v in ["1.14.2", "1.14.1", "1.14.0", "1.13.0", "1.12.0"] {
            if let Ok((m, _)) = sources::thunderstore::pinned(&ctx, "Lakatrazz", "Fusion", Some(v)).await {
                older = Some(m);
                break;
            }
        }
        let older = older.expect("an older Fusion version");
        assert_eq!(older.target, "code");
        assert!(db::insert_mod(&db, bonelab, &older, "ramon").await.unwrap());
        db::record(&db, bonelab, "ramon", "add", &older.name, &older.version).await.unwrap();
        let heard = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await.expect("heard live").unwrap();
        assert!(heard.contains(&format!("\"game\":{bonelab}")) && heard.contains("\"add\""), "{heard}");

        // cedric: a BepInEx mod on Lethal Company; BepInExPack comes along as a dependency
        let lg = db::game(&db, lethal).await.unwrap();
        let cs = cedric.settings.read().await.clone();
        let lctx = cedric.ctx(&cs, &lg);
        let resolved = sources::resolve(&lctx, "https://thunderstore.io/c/lethal-company/p/notnotnotswipez/MoreCompany/", &AddOptions { name: String::new(), target: "content".into() })
            .await
            .unwrap();
        let dep_ids: Vec<&String> = resolved.deps.iter().map(|d| &d.source_id).collect();
        assert!(dep_ids.iter().any(|d| d.to_lowercase().contains("bepinexpack")), "{dep_ids:?}");
        for m in std::iter::once(&resolved.main).chain(resolved.deps.iter()) {
            db::insert_mod(&db, lethal, m, "cedric").await.unwrap();
        }

        // cancelling: a download stops mid-stream…
        let fusion = sources::fetch(&ctx, &older).await.unwrap();
        let started = AtomicBool::new(false);
        let partial = ramon.temp.join("cancel-test");
        let r = install::download(fusion, &partial, |_, _| started.store(true, Ordering::SeqCst), || started.load(Ordering::SeqCst)).await;
        assert_eq!(r.err().as_deref(), Some(install::CANCELLED), "stops after the first chunk");
        let _ = std::fs::remove_file(&partial);
        // …and cancelled jobs install nothing; the next sync queues them again
        enqueue(&ramon, lethal).await.unwrap();
        let ids: Vec<u64> = ramon.queue.snapshot().iter().map(|j| j.id).collect();
        assert!(ids.len() >= 2);
        // the first one is still queued: it just leaves the queue
        ramon.queue.cancel(ids[0]).unwrap();
        assert!(ramon.queue.get(ids[0]).is_none());
        // the others as if they were already downloading
        ramon.queue.jobs.lock().unwrap().iter_mut().for_each(|j| j.status = "downloading");
        for id in &ids[1..] {
            ramon.queue.cancel(*id).unwrap();
        }
        ramon.queue.jobs.lock().unwrap().iter_mut().for_each(|j| j.status = "queued");
        drain(&ramon, &|_| {}).await;
        assert!(ramon.queue.snapshot().iter().all(|j| j.status == "cancelled"), "{:?}", ramon.queue.snapshot().iter().map(|j| j.status).collect::<Vec<_>>());
        assert!(ramon.local.lock().await.game(lethal).installed.is_empty(), "nothing installed");
        assert!(ramon.queue.cancel.lock().unwrap().is_empty(), "cancel requests are cleared");
        ramon.queue.jobs.lock().unwrap().clear();

        // all three sync both games
        for c in [&ramon, &cedric, &alex] {
            assert_eq!(sync(c, bonelab).await, vec!["install"]);
            let actions = sync(c, lethal).await;
            assert!(actions.len() >= 2 && actions.iter().all(|a| *a == "install"), "{actions:?}");
            assert!(sync(c, bonelab).await.is_empty() && sync(c, lethal).await.is_empty());
        }
        let alex_paths = alex.settings.read().await.paths(lethal);
        assert!(install::loader_installed(&alex_paths, "bepinex"), "BepInExPack landed in the game folder");
        let alex_local = alex.local.lock().await.game(lethal);
        let more = alex_local.installed.get(&resolved.main.key()).unwrap();
        assert!(more.paths.iter().all(|p| p.starts_with("BepInEx/")), "{:?}", more.paths);
        assert!(install::unmanaged(&alex_paths, "bepinex", &alex_local).is_empty(), "everything installed is accounted for");

        // ramon updates Fusion: the other two get the update, Lethal Company stays untouched
        let id = db::list_mods(&db, bonelab).await.unwrap()[0].id;
        let cur = db::get_mod(&db, id).await.unwrap();
        let next = sources::latest(&ctx, &cur.data, true).await.unwrap().expect("newer Fusion");
        db::repin(&db, id, &next, "ramon").await.unwrap();
        for c in [&cedric, &alex] {
            assert_eq!(sync(c, bonelab).await, vec!["update"]);
            assert!(sync(c, lethal).await.is_empty());
            assert_eq!(c.local.lock().await.game(bonelab).installed.values().next().unwrap().version, latest.version);
        }

        // presence: three people, each with their own state in BONELAB
        for (who, pending) in [("ramon", 1), ("cedric", 0), ("alex", 0)] {
            db::heartbeat(&db, who, bonelab, pending, pending == 0).await.unwrap();
        }
        let members = db::members(&db, bonelab).await.unwrap();
        assert_eq!(members.len(), 3);
        assert_eq!(members.iter().find(|m| m.name == "ramon").unwrap().pending, Some(1));

        // removing a game clears its list for everyone
        db::delete_game(&db, lethal).await.unwrap();
        assert!(db::list_mods(&db, lethal).await.unwrap().is_empty());
        assert_eq!(db::games(&db).await.unwrap().len(), 1);
    }

    #[test]
    fn reordering_moves_only_queued_jobs() {
        let q = Queue::default();
        let job = |id: u64, status: &'static str| Job {
            id,
            game: 1,
            game_name: "G".into(),
            key: format!("k{id}"),
            mod_id: Some(id as i64),
            name: format!("m{id}"),
            action: "install",
            status,
            done: 0,
            total: 0,
            error: None,
            manual_url: None,
            nexus: None,
        };
        *q.jobs.lock().unwrap() = vec![job(1, "done"), job(2, "queued"), job(3, "downloading"), job(4, "queued"), job(5, "queued")];
        q.reorder(&[5, 2]);
        let ids: Vec<u64> = q.snapshot().iter().map(|j| j.id).collect();
        // 5 first, then 2, then 4 (not listed); 1 and 3 stay where they were
        assert_eq!(ids, vec![1, 5, 3, 2, 4]);
        let mut touched = Vec::new();
        assert_eq!(start_next(&q, &mut touched), Some(5));
        assert_eq!(q.get(5).unwrap().status, "downloading");
    }

    #[test]
    fn adopted_files_keep_to_one_folder() {
        assert_eq!(adopted_paths(&["content/A.Pallet".into(), "content/B".into()]).unwrap(), (Root::Content, vec!["A.Pallet".into(), "B".into()]));
        assert_eq!(adopted_paths(&["game/BepInEx/plugins/Ns-Cool".into()]).unwrap(), (Root::Game, vec!["BepInEx/plugins/Ns-Cool".into()]));
        assert!(adopted_paths(&["content/A".into(), "game/Mods/B.dll".into()]).is_err(), "not both folders");
        assert!(adopted_paths(&["content/../evil".into()]).is_err());
        assert!(adopted_paths(&[]).is_err());
    }

    #[test]
    fn invite_codes_round_trip() {
        let code = format!(
            "{INVITE_PREFIX}{}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(serde_json::to_vec(&Invite { database_url: "postgres://a:b@c/d".into(), game: Some(3) }).unwrap())
        );
        let back = read_invite(code).unwrap();
        assert_eq!((back.database_url.as_str(), back.game), ("postgres://a:b@c/d", Some(3)));
        assert!(read_invite("nope".into()).is_err());
    }
}
