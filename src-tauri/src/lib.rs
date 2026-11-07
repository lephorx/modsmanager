//! Mods Manager: one shared list of exact mod files, and a Sync button that makes
//! this computer match it.

mod config;
mod db;
mod install;
mod sources;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_opener::OpenerExt;
use tokio::sync::{Mutex, RwLock};
use tokio_postgres::Client;

use config::{Installed, Local, Root, Settings};
use db::{Event, Member, NewMod, SharedMod};
use sources::{AddOptions, Ctx, Resolved, SearchHit};

type R<T> = Result<T, String>;

pub struct AppState {
    data_dir: PathBuf,
    temp: PathBuf,
    settings: RwLock<Settings>,
    local: Mutex<Local>,
    db: Mutex<Option<Arc<Client>>>,
    http: reqwest::Client,
    sync_lock: Mutex<()>,
    listener: std::sync::Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    /// Live updates are connected (also sent as the `live` event).
    live: AtomicBool,
}

impl AppState {
    fn settings_path(&self) -> PathBuf {
        self.data_dir.join("settings.json")
    }
    fn local_path(&self) -> PathBuf {
        self.data_dir.join("local.json")
    }

    async fn db(&self) -> R<Arc<Client>> {
        let mut slot = self.db.lock().await;
        if let Some(c) = slot.as_ref() {
            if !c.is_closed() {
                return Ok(c.clone());
            }
        }
        let url = self.settings.read().await.database_url.clone();
        if url.trim().is_empty() {
            return Err("Set up the shared database in Settings first".into());
        }
        let c = Arc::new(db::connect(&url).await?);
        *slot = Some(c.clone());
        Ok(c)
    }

    async fn save_local(&self, local: &Local) -> R<()> {
        config::save(&self.local_path(), local)
    }

    async fn me(&self) -> String {
        self.settings.read().await.name.trim().to_string()
    }
}

// ── the plan: what this computer has to do to match the list ──────────────

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
}

fn state_of(settings: &Settings, local: &Local, m: &SharedMod) -> &'static str {
    match local.installed.get(&m.data.key()) {
        None => "missing",
        Some(i) if i.pin != m.data.pin() => "outdated",
        Some(i) if !install::is_present(settings, i) => "broken",
        Some(_) => "installed",
    }
}

fn plan(settings: &Settings, local: &Local, mods: &[SharedMod]) -> Vec<PlanItem> {
    let mut out = Vec::new();
    for m in mods {
        let inst = local.installed.get(&m.data.key());
        let action = match state_of(settings, local, m) {
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
            });
        }
    }
    out
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    me: String,
    mods: Vec<ModView>,
    plan: Vec<PlanItem>,
    members: Vec<Member>,
    unmanaged: usize,
    melon: bool,
    needs_melon: bool,
    live: bool,
}

// ── commands: settings ─────────────────────────────────────────────────────

#[tauri::command]
async fn get_settings(state: State<'_, AppState>) -> R<Settings> {
    Ok(state.settings.read().await.clone())
}

#[tauri::command]
async fn save_settings(app: AppHandle, state: State<'_, AppState>, settings: Settings) -> R<()> {
    let url_changed = state.settings.read().await.database_url != settings.database_url;
    config::save(&state.settings_path(), &settings)?;
    *state.settings.write().await = settings;
    if url_changed {
        *state.db.lock().await = None;
        start_listener(&app);
    }
    Ok(())
}

#[tauri::command]
fn detect_folders() -> config::Detected {
    config::detect()
}

#[tauri::command]
async fn test_connection(url: String) -> R<()> {
    db::connect(&url).await.map(|_| ())
}

// ── commands: the list ─────────────────────────────────────────────────────

#[tauri::command]
async fn overview(state: State<'_, AppState>) -> R<Overview> {
    let db = state.db().await?;
    let settings = state.settings.read().await.clone();
    let mods = db::list_mods(&db).await?;
    let local = state.local.lock().await.clone();
    let plan = plan(&settings, &local, &mods);
    let me = settings.name.trim().to_string();
    if !me.is_empty() {
        db::heartbeat(&db, &me, plan.len() as i32, plan.is_empty()).await?;
    }
    let members = db::members(&db).await?;
    let views = mods
        .into_iter()
        .map(|m| {
            let st = state_of(&settings, &local, &m);
            let local_version = local.installed.get(&m.data.key()).map(|i| i.version.clone());
            ModView { m, state: st, local_version }
        })
        .collect::<Vec<_>>();
    let needs_melon = views.iter().any(|v| v.m.data.target == "code");
    Ok(Overview {
        me,
        unmanaged: install::unmanaged(&settings, &local).len(),
        melon: install::melon_installed(&settings),
        needs_melon,
        live: state.live.load(Ordering::Relaxed),
        mods: views,
        plan,
        members,
    })
}

#[tauri::command]
async fn resolve_mod(state: State<'_, AppState>, input: String, name: Option<String>, target: Option<String>) -> R<Resolved> {
    let settings = state.settings.read().await.clone();
    let ctx = Ctx { http: &state.http, settings: &settings, temp: &state.temp };
    let opts = AddOptions { name: name.unwrap_or_default(), target: target.unwrap_or_else(|| "content".into()) };
    sources::resolve(&ctx, &input, &opts).await
}

#[tauri::command]
async fn search_modio(state: State<'_, AppState>, query: String) -> R<Vec<SearchHit>> {
    let settings = state.settings.read().await.clone();
    let ctx = Ctx { http: &state.http, settings: &settings, temp: &state.temp };
    sources::modio::search(&ctx, &query).await
}

/// Puts mods on the shared list; returns how many were new.
#[tauri::command]
async fn add_mods(state: State<'_, AppState>, mods: Vec<NewMod>) -> R<usize> {
    let db = state.db().await?;
    let me = state.me().await;
    let mut added = 0;
    for m in &mods {
        if db::insert_mod(&db, m, &me).await? {
            added += 1;
            db::record(&db, &me, "add", &m.name, &m.version).await?;
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
    db::record(&db, &me, "remove", &m.data.name, &m.data.version).await
}

/// Newer files for the mods on the list (links aren't checked: that would download them all).
#[tauri::command]
async fn check_updates(state: State<'_, AppState>) -> R<HashMap<i64, NewMod>> {
    let db = state.db().await?;
    let settings = state.settings.read().await.clone();
    let mods = db::list_mods(&db).await?;
    // each lookup owns what it needs: borrowed futures in a stream trip up the command's Send bound
    let settings = Arc::new(settings);
    let lookups = mods.into_iter().map(|m| {
        let (http, settings, temp) = (state.http.clone(), settings.clone(), state.temp.clone());
        tauri::async_runtime::spawn(async move {
            let ctx = Ctx { http: &http, settings: &settings, temp: &temp };
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

/// Re-pins a mod to its newest file (or the one given), for everyone.
#[tauri::command]
async fn update_mod(state: State<'_, AppState>, id: i64) -> R<Option<String>> {
    let db = state.db().await?;
    let settings = state.settings.read().await.clone();
    let me = settings.name.trim().to_string();
    let cur = db::get_mod(&db, id).await?;
    let ctx = Ctx { http: &state.http, settings: &settings, temp: &state.temp };
    let Some(next) = sources::latest(&ctx, &cur.data, true).await? else { return Ok(None) };
    db::repin(&db, id, &next, &me).await?;
    db::record(&db, &me, "update", &next.name, &format!("{} -> {}", cur.data.version, next.version)).await?;
    Ok(Some(next.version))
}

#[tauri::command]
async fn events(state: State<'_, AppState>) -> R<Vec<Event>> {
    let db = state.db().await?;
    db::events(&db, 300).await
}

// ── commands: sync ─────────────────────────────────────────────────────────

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Progress {
    index: usize,
    count: usize,
    name: String,
    action: &'static str,
    /// downloading | installing | removing | done
    phase: &'static str,
    done: u64,
    total: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncResult {
    done: usize,
    failed: Vec<(String, String)>,
}

async fn install_one(emit: &(dyn Fn(Progress) + Send + Sync), state: &AppState, settings: &Settings, m: &SharedMod, old: Option<Installed>, progress: Progress) -> R<Installed> {
    let code = m.data.target == "code";
    if code && settings.game_dir.trim().is_empty() {
        return Err("Set your BONELAB game folder in Settings (code mods go there)".into());
    }
    if !code && settings.mods_dir.trim().is_empty() {
        return Err("Set your BONELAB Mods folder in Settings".into());
    }
    let ctx = Ctx { http: &state.http, settings, temp: &state.temp };
    let fetch = sources::fetch(&ctx, &m.data).await?;
    let file = state.temp.join(format!("dl-{}", m.id));
    let staging = state.temp.join(format!("unpack-{}", m.id));
    let mut last = Instant::now() - Duration::from_secs(1);
    let got = install::download(fetch, &file, |done, total| {
        if last.elapsed() >= Duration::from_millis(120) || done == total {
            last = Instant::now();
            emit(Progress { phase: "downloading", done, total, ..progress.clone() });
        }
    })
    .await;
    let got = match got {
        Ok(g) => g,
        Err(e) => {
            let _ = std::fs::remove_file(&file);
            return Err(e);
        }
    };
    emit(Progress { phase: "installing", done: 0, total: 0, ..progress.clone() });
    let settings2 = settings.clone();
    let name = m.data.name.clone();
    let (file2, staging2) = (file.clone(), staging.clone());
    let placed = tauri::async_runtime::spawn_blocking(move || -> R<(Root, Vec<String>)> {
        install::unpack(&file2, &got.file_name, &staging2)?;
        // take the old version out first, so renamed folders don't stay behind
        if let Some(old) = &old {
            install::remove(&settings2, old)?;
        }
        if code {
            Ok((Root::Game, install::place_code(&staging2, std::path::Path::new(&settings2.game_dir))?))
        } else {
            Ok((Root::Mods, install::place_content(&staging2, std::path::Path::new(&settings2.mods_dir), &name)?))
        }
    })
    .await
    .map_err(|e| e.to_string())
    .and_then(|r| r);
    let _ = std::fs::remove_file(&file);
    let _ = std::fs::remove_dir_all(&staging);
    let (root, paths) = placed?;
    Ok(Installed { name: m.data.name.clone(), pin: m.data.pin(), version: m.data.version.clone(), root, paths, installed_at: chrono::Utc::now() })
}

#[tauri::command]
async fn sync_now(app: AppHandle, state: State<'_, AppState>) -> R<SyncResult> {
    let _running = state.sync_lock.try_lock().map_err(|_| "A sync is already running".to_string())?;
    let db = state.db().await?;
    let settings = state.settings.read().await.clone();
    let mods = db::list_mods(&db).await?;
    let items = plan(&settings, &*state.local.lock().await, &mods);
    let count = items.len();
    let app2 = app.clone();
    let emit = move |p: Progress| {
        let _ = app2.emit("sync-progress", p);
    };
    let mut result = SyncResult { done: 0, failed: vec![] };
    for (index, item) in items.iter().enumerate() {
        let progress = Progress { index, count, name: item.name.clone(), action: item.action, phase: "removing", done: 0, total: 0 };
        let old = state.local.lock().await.installed.get(&item.key).cloned();
        let outcome: R<Option<Installed>> = if item.action == "remove" {
            let _ = app.emit("sync-progress", progress.clone());
            match old {
                Some(old) => {
                    let s = settings.clone();
                    tauri::async_runtime::spawn_blocking(move || install::remove(&s, &old)).await.map_err(|e| e.to_string()).and_then(|r| r).map(|_| None)
                }
                None => Ok(None),
            }
        } else {
            match mods.iter().find(|m| Some(m.id) == item.id) {
                Some(m) => install_one(&emit, &state, &settings, m, old, progress.clone()).await.map(Some),
                None => Ok(None),
            }
        };
        match outcome {
            Ok(installed) => {
                let mut local = state.local.lock().await;
                match installed {
                    Some(i) => {
                        local.installed.insert(item.key.clone(), i);
                    }
                    None => {
                        local.installed.remove(&item.key);
                    }
                }
                state.save_local(&local).await?;
                result.done += 1;
            }
            Err(e) => result.failed.push((item.name.clone(), e)),
        }
    }
    let _ = app.emit("sync-progress", Progress { index: count, count, name: String::new(), action: "install", phase: "done", done: 0, total: 0 });
    let me = settings.name.trim();
    if !me.is_empty() {
        db::heartbeat(&db, me, result.failed.len() as i32, result.failed.is_empty()).await?;
        if count > 0 {
            db::notify(&db, me, "synced", "").await?;
        }
    }
    Ok(result)
}

// ── commands: housekeeping ─────────────────────────────────────────────────

#[tauri::command]
async fn unmanaged(state: State<'_, AppState>) -> R<Vec<install::Unmanaged>> {
    let settings = state.settings.read().await.clone();
    let local = state.local.lock().await.clone();
    Ok(install::unmanaged(&settings, &local))
}

#[tauri::command]
async fn delete_unmanaged(state: State<'_, AppState>, keys: Vec<String>) -> R<()> {
    let settings = state.settings.read().await.clone();
    for key in keys {
        install::delete_unmanaged(&settings, &key)?;
    }
    Ok(())
}

#[tauri::command]
async fn ignore_unmanaged(state: State<'_, AppState>, keys: Vec<String>, ignore: bool) -> R<()> {
    let mut local = state.local.lock().await;
    for key in keys {
        if ignore {
            local.ignored.insert(key);
        } else {
            local.ignored.remove(&key);
        }
    }
    state.save_local(&local).await
}

/// Copies the list from the old Mod.io-Mods-Manager tables (pinned to today's files).
#[tauri::command]
async fn import_legacy(state: State<'_, AppState>) -> R<(usize, Vec<String>)> {
    let db = state.db().await?;
    let settings = state.settings.read().await.clone();
    let me = settings.name.trim().to_string();
    let legacy = db::legacy(&db).await?.ok_or("This database has no tables from the old Mod.io Mods Manager")?;
    let ctx = Ctx { http: &state.http, settings: &settings, temp: &state.temp };
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
                if db::insert_mod(&db, &m, &me).await? {
                    added += 1;
                }
            }
            Err(e) => failed.push(e),
        }
    }
    if added > 0 {
        db::record(&db, &me, "import", &format!("{added} mods"), "from the old manager").await?;
    }
    Ok((added, failed))
}

#[tauri::command]
async fn open_folder(app: AppHandle, state: State<'_, AppState>, which: String) -> R<()> {
    let s = state.settings.read().await;
    let path = match which.as_str() {
        "mods" => s.mods_dir.clone(),
        "game" => s.game_dir.clone(),
        _ => return Err("Unknown folder".into()),
    };
    if path.is_empty() {
        return Err("That folder isn't set yet".into());
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

#[tauri::command]
fn app_version(app: AppHandle) -> String {
    app.package_info().version.to_string()
}

// ── live updates ───────────────────────────────────────────────────────────

/// Listens for changes to the list and forwards them to the window as `remote-changed`.
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
            let mut settings: Settings = config::load(&data_dir.join("settings.json"));
            // fill in BONELAB's folders on the first start
            if settings.mods_dir.is_empty() || settings.game_dir.is_empty() {
                let found = config::detect();
                if settings.mods_dir.is_empty() {
                    settings.mods_dir = found.mods_dir.unwrap_or_default();
                }
                if settings.game_dir.is_empty() {
                    settings.game_dir = found.game_dir.unwrap_or_default();
                }
            }
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
                sync_lock: Mutex::new(()),
                listener: std::sync::Mutex::new(None),
                live: AtomicBool::new(false),
            });
            start_listener(app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            detect_folders,
            test_connection,
            overview,
            resolve_mod,
            search_modio,
            add_mods,
            remove_mod,
            check_updates,
            update_mod,
            events,
            sync_now,
            unmanaged,
            delete_unmanaged,
            ignore_unmanaged,
            import_legacy,
            open_folder,
            open_url,
            app_version,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Mods Manager");
}

/// Two computers, one shared list, real downloads. Needs a throwaway Postgres:
/// `MM_TEST_DB=postgres://... cargo test -- --ignored two_friends`
#[cfg(test)]
mod e2e {
    use super::*;

    fn computer(name: &str, url: &str) -> AppState {
        let base = std::env::temp_dir().join(format!("mm-e2e-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        for d in ["data", "temp", "mods", "game/Mods", "game/MelonLoader"] {
            std::fs::create_dir_all(base.join(d)).unwrap();
        }
        let settings = Settings {
            name: name.into(),
            database_url: url.into(),
            mods_dir: base.join("mods").to_string_lossy().into(),
            game_dir: base.join("game").to_string_lossy().into(),
            ..Default::default()
        };
        AppState {
            data_dir: base.join("data"),
            temp: base.join("temp"),
            settings: RwLock::new(settings),
            local: Mutex::new(Local::default()),
            db: Mutex::new(None),
            http: reqwest::Client::builder().user_agent("ModsManager-test").build().unwrap(),
            sync_lock: Mutex::new(()),
            listener: std::sync::Mutex::new(None),
            live: AtomicBool::new(false),
        }
    }

    /// What `sync_now` does, minus the window events.
    async fn sync(state: &AppState) -> Vec<PlanItem> {
        let db = state.db().await.unwrap();
        let settings = state.settings.read().await.clone();
        let mods = db::list_mods(&db).await.unwrap();
        let items = plan(&settings, &*state.local.lock().await, &mods);
        for item in &items {
            let old = state.local.lock().await.installed.get(&item.key).cloned();
            if item.action == "remove" {
                install::remove(&settings, &old.unwrap()).unwrap();
                state.local.lock().await.installed.remove(&item.key);
            } else {
                let m = mods.iter().find(|m| Some(m.id) == item.id).unwrap();
                let p = Progress { index: 0, count: 1, name: m.data.name.clone(), action: item.action, phase: "downloading", done: 0, total: 0 };
                let inst = install_one(&|_| {}, state, &settings, m, old, p).await.unwrap();
                state.local.lock().await.installed.insert(item.key.clone(), inst);
            }
        }
        items
    }

    #[tokio::test]
    #[ignore]
    async fn two_friends() {
        let url = std::env::var("MM_TEST_DB").expect("MM_TEST_DB");
        let ramon = computer("ramon", &url);
        let cedric = computer("cedric", &url);
        let db = ramon.db().await.unwrap();
        db.batch_execute("TRUNCATE mm_mods, mm_events, mm_members").await.unwrap();

        // cedric's app listens for changes
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let listen_url = url.clone();
        tokio::spawn(async move {
            let _ = db::listen(&listen_url, move |p| {
                let _ = tx.send(p);
            })
            .await;
        });
        tokio::time::sleep(Duration::from_millis(500)).await;

        // ramon adds Fusion, pinned to an older version
        let rs = ramon.settings.read().await.clone();
        let ctx = Ctx { http: &ramon.http, settings: &rs, temp: &ramon.temp };
        let (latest, _) = sources::thunderstore::pinned(&ctx, "Lakatrazz", "Fusion", None).await.unwrap();
        let mut older = None;
        for v in ["1.14.2", "1.14.1", "1.14.0", "1.13.0", "1.12.0"] {
            if let Ok((m, _)) = sources::thunderstore::pinned(&ctx, "Lakatrazz", "Fusion", Some(v)).await {
                older = Some(m);
                break;
            }
        }
        let older = older.expect("an older Fusion version");
        assert_ne!(older.version, latest.version);
        assert!(db::insert_mod(&db, &older, "ramon").await.unwrap());
        db::record(&db, "ramon", "add", &older.name, &older.version).await.unwrap();

        let heard = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await.expect("cedric hears it live").unwrap();
        assert!(heard.contains("\"actor\":\"ramon\"") && heard.contains("\"add\""), "{heard}");

        // cedric syncs: Fusion gets installed, MelonLoader layout
        let done = sync(&cedric).await;
        assert_eq!(done.len(), 1);
        assert_eq!(done[0].action, "install");
        let inst = cedric.local.lock().await.installed.values().next().cloned().unwrap();
        assert_eq!(inst.version, older.version);
        assert!(inst.paths.iter().any(|p| p.starts_with("Mods/") && p.ends_with(".dll")), "{:?}", inst.paths);
        let cs = cedric.settings.read().await.clone();
        assert!(install::is_present(&cs, &inst));
        assert!(sync(&cedric).await.is_empty(), "nothing left after a sync");

        // ramon updates it for both of them
        let id = db::list_mods(&db).await.unwrap()[0].id;
        let cur = db::get_mod(&db, id).await.unwrap();
        let next = sources::latest(&ctx, &cur.data, true).await.unwrap().expect("a newer version");
        db::repin(&db, id, &next, "ramon").await.unwrap();
        let done = sync(&cedric).await;
        assert_eq!(done.len(), 1);
        assert_eq!(done[0].action, "update");
        assert_eq!(cedric.local.lock().await.installed.values().next().unwrap().version, latest.version);

        // a DLL nobody put on the list shows up as unmanaged
        std::fs::write(std::path::Path::new(&cs.game_dir).join("Mods/Stray.dll"), b"x").unwrap();
        let stray = install::unmanaged(&cs, &*cedric.local.lock().await);
        assert_eq!(stray.iter().map(|u| u.key.as_str()).collect::<Vec<_>>(), vec!["game/Mods/Stray.dll"]);

        // ramon removes it; cedric's files go on the next sync
        let paths = cedric.local.lock().await.installed.values().next().unwrap().paths.clone();
        db::delete_mod(&db, id).await.unwrap();
        let done = sync(&cedric).await;
        assert_eq!(done[0].action, "remove");
        for p in paths {
            assert!(!std::path::Path::new(&cs.game_dir).join(p).exists());
        }
        assert!(cedric.local.lock().await.installed.is_empty());

        db::heartbeat(&db, "cedric", 0, true).await.unwrap();
        let members = db::members(&db).await.unwrap();
        assert!(members.iter().any(|m| m.name == "cedric" && m.pending == 0 && m.synced_at.is_some()));
    }
}
