//! Where mods come from. Each source turns what you paste into a pinned `NewMod`,
//! finds newer files, and says how to download the pinned one.

pub mod link;
pub mod modio;
pub mod nexus;
pub mod thunderstore;

use std::collections::HashSet;

use serde::Serialize;

use crate::config::Settings;
use crate::db::{Game, NewMod};

pub struct Ctx<'a> {
    pub http: &'a reqwest::Client,
    pub settings: &'a Settings,
    pub temp: &'a std::path::Path,
    /// The game whose list this is for.
    pub game: &'a Game,
    /// One-time Nexus download keys from `nxm://` links.
    pub nxm: &'a nexus::NxmKeys,
}

impl Ctx<'_> {
    /// Where code mods from this game go: the loader's folders, or the content folder without one.
    pub fn code_target(&self) -> &'static str {
        if self.game.data.loader == "none" {
            "content"
        } else {
            "code"
        }
    }
}

/// How to download one pinned file.
pub struct Fetch {
    pub request: reqwest::RequestBuilder,
    /// Used when the server doesn't name the file.
    pub file_name: String,
    pub md5: Option<String>,
    pub sha256: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Resolved {
    pub main: NewMod,
    /// Thunderstore dependencies that come along (MelonLoader itself excluded).
    pub deps: Vec<NewMod>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub id: String,
    pub input: String,
    pub name: String,
    pub summary: String,
    pub thumb: String,
    pub author: String,
    pub downloads: i64,
}

pub struct AddOptions {
    pub name: String,
    pub target: String,
}

pub fn kind_of(input: &str) -> &'static str {
    let s = input.trim();
    if s.is_empty() {
        "unknown"
    } else if s.contains("nexusmods.com/") {
        "nexus"
    } else if s.contains("mod.io/") || s.chars().all(|c| c.is_ascii_digit()) {
        "modio"
    } else if s.contains("thunderstore.io") || (!s.contains("://") && !s.contains(' ') && s.contains('-')) {
        "thunderstore"
    } else if s.starts_with("http://") || s.starts_with("https://") {
        "link"
    } else {
        "unknown"
    }
}

pub async fn resolve(ctx: &Ctx<'_>, input: &str, opts: &AddOptions) -> Result<Resolved, String> {
    let input = input.trim();
    match kind_of(input) {
        "modio" => Ok(Resolved { main: modio::resolve(ctx, input).await?, deps: vec![] }),
        "thunderstore" => {
            let (main, deps) = thunderstore::resolve(ctx, input).await?;
            // dependencies of dependencies, each once
            let mut seen: HashSet<String> = HashSet::from([main.source_id.clone()]);
            let mut queue = deps;
            let mut out = Vec::new();
            while let Some(dep) = queue.pop() {
                if out.len() > 30 {
                    break;
                }
                let Some((ns, name, ver)) = thunderstore::parse(&dep) else { continue };
                if thunderstore::is_loader(&ns, &name) || !seen.insert(format!("{ns}/{name}")) {
                    continue;
                }
                let (m, more) = thunderstore::pinned(ctx, &ns, &name, ver.as_deref()).await?;
                queue.extend(more);
                out.push(m);
            }
            Ok(Resolved { main, deps: out })
        }
        "nexus" => Ok(Resolved { main: nexus::resolve(ctx, input, &opts.target).await?, deps: vec![] }),
        "link" => Ok(Resolved { main: link::resolve(ctx, input, opts).await?, deps: vec![] }),
        _ => Err("Paste a mod.io, Thunderstore or Nexus Mods link, a mod.io id, or a direct download link".into()),
    }
}

/// A newer pin for the mod, if there is one. Links are only re-checked when `deep`
/// (that downloads the file again).
pub async fn latest(ctx: &Ctx<'_>, m: &NewMod, deep: bool) -> Result<Option<NewMod>, String> {
    match m.source.as_str() {
        "modio" => modio::latest(ctx, m).await,
        "thunderstore" => thunderstore::latest(ctx, m).await,
        "nexus" => nexus::latest(ctx, m).await,
        "link" if deep => link::latest(ctx, m).await,
        _ => Ok(None),
    }
}

pub async fn fetch(ctx: &Ctx<'_>, m: &NewMod) -> Result<Fetch, String> {
    match m.source.as_str() {
        "modio" => modio::fetch(ctx, m).await,
        "thunderstore" => Ok(thunderstore::fetch(ctx, m)),
        "nexus" => nexus::fetch(ctx, m).await,
        "link" => Ok(link::fetch(ctx, m)),
        other => Err(format!("Unknown source {other}")),
    }
}

/// The server's error text when a request fails.
pub async fn check(resp: reqwest::Response, what: &str) -> Result<reqwest::Response, String> {
    if resp.status().is_success() {
        return Ok(resp);
    }
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    let msg = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| v["error"]["message"].as_str().or(v["detail"].as_str()).map(str::to_string))
        .unwrap_or_else(|| body.chars().take(200).collect());
    Err(format!("{what}: {status} {msg}").trim().to_string())
}

pub fn s(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}
