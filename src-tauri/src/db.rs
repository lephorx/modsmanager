//! The shared lists: one Postgres database everyone connects to, holding any number of games,
//! each with its own mod list.
//!
//! Every mod pins one exact file (a mod.io modfile, a Thunderstore version or a file hash),
//! so "in sync" means byte-for-byte the same mods, not just "the latest of each".
//! Every change sends `NOTIFY mm_changed`, which everyone's app hears right away.

use chrono::{DateTime, Utc};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio_postgres::{AsyncMessage, Client, Row};

pub const CHANNEL: &str = "mm_changed";
/// BONELAB on mod.io: v0.1.0's only game, which old lists migrate into.
pub const BONELAB_MODIO: i64 = 3809;

/// Fresh databases get everything; older ones are brought up to date (see `migrate`).
const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS mm_games (
    id            BIGSERIAL PRIMARY KEY,
    name          TEXT NOT NULL,
    modio_id      BIGINT NOT NULL DEFAULT 0,
    modio_slug    TEXT NOT NULL DEFAULT '',
    thunderstore  TEXT NOT NULL DEFAULT '',
    loader        TEXT NOT NULL DEFAULT 'none',
    steam_dir     TEXT NOT NULL DEFAULT '',
    content_hint  TEXT NOT NULL DEFAULT '',
    created_by    TEXT NOT NULL DEFAULT '',
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE IF NOT EXISTS mm_mods (
    id          BIGSERIAL PRIMARY KEY,
    source      TEXT NOT NULL,
    source_id   TEXT NOT NULL,
    target      TEXT NOT NULL DEFAULT 'content',
    name        TEXT NOT NULL,
    version     TEXT NOT NULL DEFAULT '',
    file_ref    TEXT NOT NULL DEFAULT '',
    file_hash   TEXT NOT NULL DEFAULT '',
    file_size   BIGINT NOT NULL DEFAULT 0,
    page_url    TEXT NOT NULL DEFAULT '',
    thumb_url   TEXT NOT NULL DEFAULT '',
    summary     TEXT NOT NULL DEFAULT '',
    author      TEXT NOT NULL DEFAULT '',
    extra       JSONB NOT NULL DEFAULT '{}',
    added_by    TEXT NOT NULL,
    added_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_by  TEXT NOT NULL,
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE IF NOT EXISTS mm_events (
    id        BIGSERIAL PRIMARY KEY,
    at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    actor     TEXT NOT NULL,
    action    TEXT NOT NULL,
    mod_name  TEXT NOT NULL,
    detail    TEXT NOT NULL DEFAULT ''
);
-- per game, per person: who's online and how far behind (v0.1.0's mm_members stays for old apps)
CREATE TABLE IF NOT EXISTS mm_presence (
    name       TEXT NOT NULL,
    game_id    BIGINT NOT NULL,
    last_seen  TIMESTAMPTZ NOT NULL DEFAULT now(),
    pending    INT NOT NULL DEFAULT 0,
    synced_at  TIMESTAMPTZ,
    PRIMARY KEY (name, game_id)
);
-- v0.1.0 → games
ALTER TABLE mm_mods ADD COLUMN IF NOT EXISTS game_id BIGINT;
ALTER TABLE mm_events ADD COLUMN IF NOT EXISTS game_id BIGINT;
ALTER TABLE mm_games ADD COLUMN IF NOT EXISTS nexus TEXT NOT NULL DEFAULT '';
ALTER TABLE mm_mods DROP CONSTRAINT IF EXISTS mm_mods_source_source_id_key;
CREATE UNIQUE INDEX IF NOT EXISTS mm_mods_game_source ON mm_mods (game_id, source, source_id);
CREATE INDEX IF NOT EXISTS mm_events_game ON mm_events (game_id, id DESC);
"#;

/// A game with its own shared mod list.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Game {
    pub id: i64,
    #[serde(flatten)]
    pub data: NewGame,
    pub created_by: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct NewGame {
    pub name: String,
    /// mod.io game id (0: not on mod.io)
    pub modio_id: i64,
    /// mod.io's name for it, as in mod.io/g/<slug>
    pub modio_slug: String,
    /// Thunderstore community, as in thunderstore.io/c/<community>
    pub thunderstore: String,
    /// Nexus Mods game, as in nexusmods.com/<game>
    pub nexus: String,
    /// How code mods are loaded: `melonloader`, `bepinex` or `none`
    pub loader: String,
    /// Its folder under steamapps/common, to find it
    pub steam_dir: String,
    /// Where content mods go: a path, or one starting with %GAME%, %LOCALLOW% or %DOCUMENTS%
    pub content_hint: String,
}

pub fn presets() -> Vec<NewGame> {
    vec![
        NewGame {
            name: "BONELAB".into(),
            modio_id: BONELAB_MODIO,
            modio_slug: "bonelab".into(),
            thunderstore: "bonelab".into(),
            nexus: "bonelab".into(),
            loader: "melonloader".into(),
            steam_dir: "BONELAB".into(),
            content_hint: "%LOCALLOW%/Stress Level Zero/BONELAB/Mods".into(),
        },
        NewGame {
            name: "Blade & Sorcery".into(),
            modio_id: 0,
            modio_slug: "bladeandsorcery".into(),
            thunderstore: String::new(),
            nexus: "bladeandsorcery".into(),
            loader: "none".into(),
            steam_dir: "Blade & Sorcery".into(),
            content_hint: "%GAME%/BladeAndSorcery_Data/StreamingAssets/Mods".into(),
        },
        NewGame {
            name: "Lethal Company".into(),
            modio_id: 0,
            modio_slug: String::new(),
            thunderstore: "lethal-company".into(),
            nexus: "lethalcompany".into(),
            loader: "bepinex".into(),
            steam_dir: "Lethal Company".into(),
            content_hint: String::new(),
        },
    ]
}

/// A mod on a shared list.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SharedMod {
    pub id: i64,
    pub game_id: i64,
    #[serde(flatten)]
    pub data: NewMod,
    pub added_by: String,
    pub added_at: DateTime<Utc>,
    pub updated_by: String,
    pub updated_at: DateTime<Utc>,
}

/// Everything needed to add (or re-pin) a mod.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct NewMod {
    /// `modio`, `thunderstore` or `link`
    pub source: String,
    /// mod.io mod id, Thunderstore `namespace/name`, or the link
    pub source_id: String,
    /// `content` (the game's content folder) or `code` (the mod loader, in the game folder)
    pub target: String,
    pub name: String,
    /// What people read: "1.2.0"
    pub version: String,
    /// What pins the file: mod.io modfile id, Thunderstore version, or "" for links
    pub file_ref: String,
    /// md5 (mod.io) or sha256 (links); empty when the source has none
    pub file_hash: String,
    pub file_size: i64,
    pub page_url: String,
    pub thumb_url: String,
    pub summary: String,
    pub author: String,
    /// Source-specific extras (unused for now, kept so the schema can grow)
    pub extra: serde_json::Value,
}

impl NewMod {
    pub fn key(&self) -> String {
        format!("{}:{}", self.source, self.source_id)
    }
    /// Two installs with the same pin are the same bytes.
    pub fn pin(&self) -> String {
        format!("{}|{}", self.file_ref, self.file_hash)
    }
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub id: i64,
    pub at: DateTime<Utc>,
    pub actor: String,
    pub action: String,
    pub mod_name: String,
    pub detail: String,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Member {
    pub name: String,
    /// Seen in any game
    pub last_seen: DateTime<Utc>,
    /// Changes to sync in this game; None when they haven't opened it
    pub pending: Option<i32>,
    pub synced_at: Option<DateTime<Utc>>,
}

fn tls() -> Result<postgres_native_tls::MakeTlsConnector, String> {
    // like libpq's sslmode=require: encrypt, but accept the self-signed certificates home servers use
    let connector = native_tls::TlsConnector::builder()
        .danger_accept_invalid_certs(true)
        .danger_accept_invalid_hostnames(true)
        .build()
        .map_err(|e| e.to_string())?;
    Ok(postgres_native_tls::MakeTlsConnector::new(connector))
}

type Connection = tokio_postgres::Connection<tokio_postgres::Socket, postgres_native_tls::TlsStream<tokio_postgres::Socket>>;

async fn open(url: &str) -> Result<(Client, Connection), String> {
    let mut config: tokio_postgres::Config = url.trim().parse().map_err(|_| "That isn't a valid database address (postgres://user:password@host:5432/db)".to_string())?;
    // A connection that died silently (sleep, network change, idle drop) otherwise looks open
    // forever and every query on it hangs; keepalive probes turn that into an error in ~1 min.
    config
        .keepalives(true)
        .keepalives_idle(std::time::Duration::from_secs(20))
        .keepalives_interval(std::time::Duration::from_secs(5))
        .keepalives_retries(4);
    let fut = config.connect(tls()?);
    match tokio::time::timeout(std::time::Duration::from_secs(10), fut).await {
        Ok(Ok(pair)) => Ok(pair),
        // the driver's message can include the address; keep the password out of the window
        Ok(Err(e)) => Err(format!("Can't reach the database: {}", scrub(&e.to_string(), url))),
        Err(_) => Err("Can't reach the database: timed out after 10 s".into()),
    }
}

fn scrub(msg: &str, url: &str) -> String {
    let mut out = msg.replace(url.trim(), "<database>");
    if let Ok(u) = url::Url::parse(url.trim()) {
        if let Some(pw) = u.password().filter(|p| !p.is_empty()) {
            out = out.replace(pw, "•••");
        }
    }
    out
}

/// A client for queries, with the schema created and old lists migrated.
pub async fn connect(url: &str) -> Result<Client, String> {
    let (mut client, conn) = open(url).await?;
    tokio::spawn(async move {
        let _ = conn.await;
    });
    migrate(&mut client).await?;
    Ok(client)
}

/// Creates the tables, and moves a v0.1.0 list (mods without a game) into a BONELAB game.
/// Runs on every connect: an old app that's still running may add game-less mods until it
/// updates itself. One app at a time, thanks to the advisory lock.
async fn migrate(client: &mut Client) -> Result<(), String> {
    let tx = client.transaction().await.map_err(err)?;
    tx.execute("SELECT pg_advisory_xact_lock(727274)", &[]).await.map_err(err)?;
    tx.batch_execute(SCHEMA).await.map_err(err)?;
    let orphans: i64 = tx
        .query_one(
            "SELECT (SELECT count(*) FROM mm_mods WHERE game_id IS NULL) + (SELECT count(*) FROM mm_events WHERE game_id IS NULL)",
            &[],
        )
        .await
        .map_err(err)?
        .get(0);
    if orphans > 0 {
        let id = ensure_bonelab(&tx).await?;
        // the same mod added again by an old app: keep the one already in the game
        tx.execute(
            "DELETE FROM mm_mods o WHERE o.game_id IS NULL AND EXISTS
               (SELECT 1 FROM mm_mods m WHERE m.game_id = $1 AND m.source = o.source AND m.source_id = o.source_id)",
            &[&id],
        )
        .await
        .map_err(err)?;
        tx.execute("UPDATE mm_mods SET game_id = $1 WHERE game_id IS NULL", &[&id]).await.map_err(err)?;
        tx.execute("UPDATE mm_events SET game_id = $1 WHERE game_id IS NULL", &[&id]).await.map_err(err)?;
        // v0.1.0's people become BONELAB's
        let had_members: bool = tx.query_one("SELECT to_regclass('public.mm_members') IS NOT NULL", &[]).await.map_err(err)?.get(0);
        if had_members {
            tx.execute(
                "INSERT INTO mm_presence (name, game_id, last_seen, pending, synced_at)
                 SELECT name, $1, last_seen, pending, synced_at FROM mm_members ON CONFLICT DO NOTHING",
                &[&id],
            )
            .await
            .map_err(err)?;
        }
    }
    tx.commit().await.map_err(err)
}

/// The BONELAB game (by its mod.io id), created from the preset if there isn't one.
async fn ensure_bonelab(db: &impl tokio_postgres::GenericClient) -> Result<i64, String> {
    if let Some(row) = db.query_opt("SELECT id FROM mm_games WHERE modio_id = $1 ORDER BY id LIMIT 1", &[&BONELAB_MODIO]).await.map_err(err)? {
        return Ok(row.get(0));
    }
    let preset = presets().into_iter().next().expect("BONELAB preset");
    insert_game_on(db, &preset, "").await
}

/// The game v0.1.0's local files belong to (created if needed).
pub async fn bonelab(db: &Client) -> Result<i64, String> {
    ensure_bonelab(db).await
}

/// Calls `on_change(payload)` for every change anyone makes, until the connection drops.
pub async fn listen(url: &str, mut on_change: impl FnMut(String) + Send) -> Result<(), String> {
    let (client, mut conn) = open(url).await?;
    let mut messages = futures_util::stream::poll_fn(move |cx| conn.poll_message(cx));
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let pump = tokio::spawn(async move {
        while let Some(msg) = messages.next().await {
            match msg {
                Ok(AsyncMessage::Notification(n)) => {
                    let _ = tx.send(n.payload().to_string());
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
    });
    client.batch_execute(&format!("LISTEN {CHANNEL}")).await.map_err(err)?;
    while let Some(payload) = rx.recv().await {
        on_change(payload);
    }
    pump.abort();
    Err("Live updates disconnected".into())
}

pub fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

// ── games ──────────────────────────────────────────────────────────────────

const GAME_COLUMNS: &str = "id, name, modio_id, modio_slug, thunderstore, loader, steam_dir, content_hint, created_by, nexus";

fn game_from(r: &Row) -> Game {
    Game {
        id: r.get(0),
        data: NewGame {
            name: r.get(1),
            modio_id: r.get(2),
            modio_slug: r.get(3),
            thunderstore: r.get(4),
            loader: r.get(5),
            steam_dir: r.get(6),
            content_hint: r.get(7),
            nexus: r.get(9),
        },
        created_by: r.get(8),
    }
}

pub async fn games(db: &Client) -> Result<Vec<Game>, String> {
    let rows = db.query(&format!("SELECT {GAME_COLUMNS} FROM mm_games ORDER BY lower(name)"), &[]).await.map_err(err)?;
    Ok(rows.iter().map(game_from).collect())
}

pub async fn game(db: &Client, id: i64) -> Result<Game, String> {
    let row = db
        .query_opt(&format!("SELECT {GAME_COLUMNS} FROM mm_games WHERE id = $1"), &[&id])
        .await
        .map_err(err)?
        .ok_or("That game isn't in the shared database anymore")?;
    Ok(game_from(&row))
}

async fn insert_game_on(db: &impl tokio_postgres::GenericClient, g: &NewGame, by: &str) -> Result<i64, String> {
    let row = db
        .query_one(
            "INSERT INTO mm_games (name, modio_id, modio_slug, thunderstore, loader, steam_dir, content_hint, created_by, nexus)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9) RETURNING id",
            &[&g.name, &g.modio_id, &g.modio_slug, &g.thunderstore, &g.loader, &g.steam_dir, &g.content_hint, &by, &g.nexus],
        )
        .await
        .map_err(err)?;
    Ok(row.get(0))
}

pub async fn insert_game(db: &Client, g: &NewGame, by: &str) -> Result<i64, String> {
    insert_game_on(db, g, by).await
}

pub async fn update_game(db: &Client, id: i64, g: &NewGame) -> Result<(), String> {
    db.execute(
        "UPDATE mm_games SET name=$2, modio_id=$3, modio_slug=$4, thunderstore=$5, loader=$6, steam_dir=$7, content_hint=$8, nexus=$9 WHERE id=$1",
        &[&id, &g.name, &g.modio_id, &g.modio_slug, &g.thunderstore, &g.loader, &g.steam_dir, &g.content_hint, &g.nexus],
    )
    .await
    .map_err(err)?;
    Ok(())
}

/// Removes the game and its list for everyone.
pub async fn delete_game(db: &Client, id: i64) -> Result<(), String> {
    db.batch_execute("BEGIN").await.map_err(err)?;
    let r = async {
        db.execute("DELETE FROM mm_mods WHERE game_id = $1", &[&id]).await?;
        db.execute("DELETE FROM mm_events WHERE game_id = $1", &[&id]).await?;
        db.execute("DELETE FROM mm_presence WHERE game_id = $1", &[&id]).await?;
        db.execute("DELETE FROM mm_games WHERE id = $1", &[&id]).await
    }
    .await;
    match r {
        Ok(_) => db.batch_execute("COMMIT").await.map_err(err),
        Err(e) => {
            let _ = db.batch_execute("ROLLBACK").await;
            Err(err(e))
        }
    }
}

pub async fn mod_counts(db: &Client) -> Result<std::collections::HashMap<i64, i64>, String> {
    let rows = db.query("SELECT game_id, count(*) FROM mm_mods WHERE game_id IS NOT NULL GROUP BY game_id", &[]).await.map_err(err)?;
    Ok(rows.iter().map(|r| (r.get(0), r.get(1))).collect())
}

// ── mods ───────────────────────────────────────────────────────────────────

const MOD_COLUMNS: &str = "id, game_id, source, source_id, target, name, version, file_ref, file_hash, file_size, page_url, thumb_url, summary, author, extra, added_by, added_at, updated_by, updated_at";

fn mod_from(row: &Row) -> SharedMod {
    SharedMod {
        id: row.get(0),
        game_id: row.get(1),
        data: NewMod {
            source: row.get(2),
            source_id: row.get(3),
            target: row.get(4),
            name: row.get(5),
            version: row.get(6),
            file_ref: row.get(7),
            file_hash: row.get(8),
            file_size: row.get(9),
            page_url: row.get(10),
            thumb_url: row.get(11),
            summary: row.get(12),
            author: row.get(13),
            extra: row.get(14),
        },
        added_by: row.get(15),
        added_at: row.get(16),
        updated_by: row.get(17),
        updated_at: row.get(18),
    }
}

pub async fn list_mods(db: &Client, game: i64) -> Result<Vec<SharedMod>, String> {
    let rows = db
        .query(&format!("SELECT {MOD_COLUMNS} FROM mm_mods WHERE game_id = $1 ORDER BY lower(name)"), &[&game])
        .await
        .map_err(err)?;
    Ok(rows.iter().map(mod_from).collect())
}

pub async fn get_mod(db: &Client, id: i64) -> Result<SharedMod, String> {
    let row = db
        .query_opt(&format!("SELECT {MOD_COLUMNS} FROM mm_mods WHERE id = $1"), &[&id])
        .await
        .map_err(err)?
        .ok_or("That mod isn't on the list anymore")?;
    Ok(mod_from(&row))
}

/// Adds the mod unless it's on the game's list already; true if it was added.
pub async fn insert_mod(db: &Client, game: i64, m: &NewMod, by: &str) -> Result<bool, String> {
    let n = db
        .execute(
            "INSERT INTO mm_mods (game_id, source, source_id, target, name, version, file_ref, file_hash, file_size, page_url, thumb_url, summary, author, extra, added_by, updated_by)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$15)
             ON CONFLICT (game_id, source, source_id) DO NOTHING",
            &[&game, &m.source, &m.source_id, &m.target, &m.name, &m.version, &m.file_ref, &m.file_hash, &m.file_size, &m.page_url, &m.thumb_url, &m.summary, &m.author, &m.extra, &by],
        )
        .await
        .map_err(err)?;
    Ok(n > 0)
}

/// Points the mod at another exact file (an update), keeping who added it.
pub async fn repin(db: &Client, id: i64, m: &NewMod, by: &str) -> Result<(), String> {
    db.execute(
        "UPDATE mm_mods SET name=$2, version=$3, file_ref=$4, file_hash=$5, file_size=$6, thumb_url=$7, summary=$8, updated_by=$9, updated_at=now() WHERE id=$1",
        &[&id, &m.name, &m.version, &m.file_ref, &m.file_hash, &m.file_size, &m.thumb_url, &m.summary, &by],
    )
    .await
    .map_err(err)?;
    Ok(())
}

pub async fn delete_mod(db: &Client, id: i64) -> Result<(), String> {
    db.execute("DELETE FROM mm_mods WHERE id = $1", &[&id]).await.map_err(err)?;
    Ok(())
}

// ── activity and presence ──────────────────────────────────────────────────

/// Logs what happened and tells everyone.
pub async fn record(db: &Client, game: i64, actor: &str, action: &str, mod_name: &str, detail: &str) -> Result<(), String> {
    db.execute(
        "INSERT INTO mm_events (game_id, actor, action, mod_name, detail) VALUES ($1,$2,$3,$4,$5)",
        &[&game, &actor, &action, &mod_name, &detail],
    )
    .await
    .map_err(err)?;
    notify(db, game, actor, action, mod_name).await
}

pub async fn notify(db: &Client, game: i64, actor: &str, action: &str, name: &str) -> Result<(), String> {
    let payload = serde_json::json!({ "game": game, "actor": actor, "action": action, "name": name }).to_string();
    db.execute("SELECT pg_notify($1, $2)", &[&CHANNEL, &payload]).await.map_err(err)?;
    Ok(())
}

pub async fn events(db: &Client, game: i64, limit: i64) -> Result<Vec<Event>, String> {
    let rows = db
        .query("SELECT id, at, actor, action, mod_name, detail FROM mm_events WHERE game_id = $1 ORDER BY id DESC LIMIT $2", &[&game, &limit])
        .await
        .map_err(err)?;
    Ok(rows
        .iter()
        .map(|r| Event { id: r.get(0), at: r.get(1), actor: r.get(2), action: r.get(3), mod_name: r.get(4), detail: r.get(5) })
        .collect())
}

/// "I'm here, in this game, this many changes behind."
pub async fn heartbeat(db: &Client, name: &str, game: i64, pending: i32, synced: bool) -> Result<(), String> {
    db.execute(
        "INSERT INTO mm_presence (name, game_id, last_seen, pending, synced_at) VALUES ($1, $2, now(), $3, CASE WHEN $4 THEN now() END)
         ON CONFLICT (name, game_id) DO UPDATE SET last_seen = now(), pending = $3,
           synced_at = CASE WHEN $4 THEN now() ELSE mm_presence.synced_at END",
        &[&name, &game, &pending, &synced],
    )
    .await
    .map_err(err)?;
    Ok(())
}

/// Everyone who has used the database, with their state in this game.
pub async fn members(db: &Client, game: i64) -> Result<Vec<Member>, String> {
    let rows = db
        .query(
            "SELECT p.name, max(p.last_seen), max(g.pending), max(g.synced_at)
             FROM mm_presence p LEFT JOIN mm_presence g ON g.name = p.name AND g.game_id = $1
             GROUP BY p.name ORDER BY lower(p.name)",
            &[&game],
        )
        .await
        .map_err(err)?;
    Ok(rows
        .iter()
        .map(|r| Member { name: r.get(0), last_seen: r.get(1), pending: r.get(2), synced_at: r.get(3) })
        .collect())
}

/// The tables of the old Mod.io-Mods-Manager (Python), if this database has them.
pub struct Legacy {
    pub modio: Vec<(i64, String)>,
    pub code: Vec<(String, String, String)>,
}

pub async fn legacy(db: &Client) -> Result<Option<Legacy>, String> {
    let exists = |t: &'static str| async move {
        db.query_one("SELECT to_regclass($1) IS NOT NULL", &[&t]).await.map(|r| r.get::<_, bool>(0)).unwrap_or(false)
    };
    let (has_mods, has_code) = (exists("public.mods").await, exists("public.code_mods").await);
    if !has_mods && !has_code {
        return Ok(None);
    }
    let mut out = Legacy { modio: vec![], code: vec![] };
    if has_mods {
        for r in db.query("SELECT mod_id::bigint, name FROM mods", &[]).await.map_err(err)? {
            out.modio.push((r.get(0), r.get(1)));
        }
    }
    if has_code {
        for r in db.query("SELECT namespace, package_name, version FROM code_mods", &[]).await.map_err(err)? {
            out.code.push((r.get(0), r.get(1), r.get(2)));
        }
    }
    Ok(Some(out))
}
