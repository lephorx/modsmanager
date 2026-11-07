//! The shared mod list: one Postgres database both of you connect to.
//!
//! Every row pins one exact file (a mod.io modfile, a Thunderstore version or a file hash),
//! so "in sync" means byte-for-byte the same mods, not just "the latest of each".
//! Every change sends `NOTIFY mm_changed`, which the other app hears right away.

use chrono::{DateTime, Utc};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio_postgres::{AsyncMessage, Client, Row};

pub const CHANNEL: &str = "mm_changed";

const SCHEMA: &str = r#"
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
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (source, source_id)
);
CREATE TABLE IF NOT EXISTS mm_events (
    id        BIGSERIAL PRIMARY KEY,
    at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    actor     TEXT NOT NULL,
    action    TEXT NOT NULL,
    mod_name  TEXT NOT NULL,
    detail    TEXT NOT NULL DEFAULT ''
);
CREATE TABLE IF NOT EXISTS mm_members (
    name       TEXT PRIMARY KEY,
    last_seen  TIMESTAMPTZ NOT NULL DEFAULT now(),
    pending    INT NOT NULL DEFAULT 0,
    synced_at  TIMESTAMPTZ
);
"#;

/// A mod on the shared list.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SharedMod {
    pub id: i64,
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
    /// `content` (BONELAB Mods folder) or `code` (MelonLoader, game folder)
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
    pub last_seen: DateTime<Utc>,
    pub pending: i32,
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
    let config: tokio_postgres::Config = url.trim().parse().map_err(|e| format!("Database address: {e}"))?;
    let fut = config.connect(tls()?);
    match tokio::time::timeout(std::time::Duration::from_secs(10), fut).await {
        Ok(Ok(pair)) => Ok(pair),
        Ok(Err(e)) => Err(format!("Can't reach the database: {e}")),
        Err(_) => Err("Can't reach the database: timed out after 10 s".into()),
    }
}

/// A client for queries, with the schema created.
pub async fn connect(url: &str) -> Result<Client, String> {
    let (client, conn) = open(url).await?;
    tokio::spawn(async move {
        let _ = conn.await;
    });
    client.batch_execute(SCHEMA).await.map_err(err)?;
    Ok(client)
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

const MOD_COLUMNS: &str = "id, source, source_id, target, name, version, file_ref, file_hash, file_size, page_url, thumb_url, summary, author, extra, added_by, added_at, updated_by, updated_at";

fn mod_from(row: &Row) -> SharedMod {
    SharedMod {
        id: row.get(0),
        data: NewMod {
            source: row.get(1),
            source_id: row.get(2),
            target: row.get(3),
            name: row.get(4),
            version: row.get(5),
            file_ref: row.get(6),
            file_hash: row.get(7),
            file_size: row.get(8),
            page_url: row.get(9),
            thumb_url: row.get(10),
            summary: row.get(11),
            author: row.get(12),
            extra: row.get(13),
        },
        added_by: row.get(14),
        added_at: row.get(15),
        updated_by: row.get(16),
        updated_at: row.get(17),
    }
}

pub async fn list_mods(db: &Client) -> Result<Vec<SharedMod>, String> {
    let rows = db
        .query(&format!("SELECT {MOD_COLUMNS} FROM mm_mods ORDER BY lower(name)"), &[])
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

/// Adds the mod unless it's on the list already; true if it was added.
pub async fn insert_mod(db: &Client, m: &NewMod, by: &str) -> Result<bool, String> {
    let n = db
        .execute(
            "INSERT INTO mm_mods (source, source_id, target, name, version, file_ref, file_hash, file_size, page_url, thumb_url, summary, author, extra, added_by, updated_by)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$14)
             ON CONFLICT (source, source_id) DO NOTHING",
            &[&m.source, &m.source_id, &m.target, &m.name, &m.version, &m.file_ref, &m.file_hash, &m.file_size, &m.page_url, &m.thumb_url, &m.summary, &m.author, &m.extra, &by],
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

/// Logs what happened and tells the other app.
pub async fn record(db: &Client, actor: &str, action: &str, mod_name: &str, detail: &str) -> Result<(), String> {
    db.execute(
        "INSERT INTO mm_events (actor, action, mod_name, detail) VALUES ($1,$2,$3,$4)",
        &[&actor, &action, &mod_name, &detail],
    )
    .await
    .map_err(err)?;
    notify(db, actor, action, mod_name).await
}

pub async fn notify(db: &Client, actor: &str, action: &str, mod_name: &str) -> Result<(), String> {
    let payload = serde_json::json!({ "actor": actor, "action": action, "name": mod_name }).to_string();
    db.execute("SELECT pg_notify($1, $2)", &[&CHANNEL, &payload]).await.map_err(err)?;
    Ok(())
}

pub async fn events(db: &Client, limit: i64) -> Result<Vec<Event>, String> {
    let rows = db
        .query("SELECT id, at, actor, action, mod_name, detail FROM mm_events ORDER BY id DESC LIMIT $1", &[&limit])
        .await
        .map_err(err)?;
    Ok(rows
        .iter()
        .map(|r| Event { id: r.get(0), at: r.get(1), actor: r.get(2), action: r.get(3), mod_name: r.get(4), detail: r.get(5) })
        .collect())
}

/// "I'm here and this many changes behind."
pub async fn heartbeat(db: &Client, name: &str, pending: i32, synced: bool) -> Result<(), String> {
    db.execute(
        "INSERT INTO mm_members (name, last_seen, pending, synced_at) VALUES ($1, now(), $2, CASE WHEN $3 THEN now() END)
         ON CONFLICT (name) DO UPDATE SET last_seen = now(), pending = $2,
           synced_at = CASE WHEN $3 THEN now() ELSE mm_members.synced_at END",
        &[&name, &pending, &synced],
    )
    .await
    .map_err(err)?;
    Ok(())
}

pub async fn members(db: &Client) -> Result<Vec<Member>, String> {
    let rows = db
        .query("SELECT name, last_seen, pending, synced_at FROM mm_members ORDER BY lower(name)", &[])
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
