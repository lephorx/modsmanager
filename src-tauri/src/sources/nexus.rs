//! Nexus Mods. File ids never change what they point to, so the file id is the pin.
//!
//! Premium accounts get download links straight from the API. Free accounts get them only with
//! the one-time key from a "Mod Manager Download" click on the website (an `nxm://` link), like
//! Vortex and Mod Organizer: the app opens the exact file's page, the user clicks, and the link
//! comes back to the app (see `NxmKey`).

use std::collections::HashMap;

use serde::Serialize;
use serde_json::Value;

use super::{check, s, Ctx, Fetch};
use crate::db::NewMod;

const API: &str = "https://api.nexusmods.com/v1";
/// Marks "a free account needs a click on the website" errors, so sync can ask instead of failing.
pub const NEEDS_CLICK: &str = "nexus-click:";

/// The one-time permission from an `nxm://` link, for one file.
#[derive(Clone, Debug)]
pub struct NxmKey {
    pub key: String,
    pub expires: i64,
}

/// `nxm://` keys that came in, by `mod_id:file_id`.
pub type NxmKeys = std::sync::Mutex<HashMap<String, NxmKey>>;

#[derive(Debug, PartialEq)]
pub struct NxmLink {
    pub domain: String,
    pub mod_id: String,
    pub file_id: String,
    pub key: Option<NxmKey>,
}

impl PartialEq for NxmKey {
    fn eq(&self, o: &Self) -> bool {
        self.key == o.key && self.expires == o.expires
    }
}

/// `nxm://bonelab/mods/123/files/456?key=…&expires=…&user_id=…`
pub fn parse_nxm(link: &str) -> Option<NxmLink> {
    let u = url::Url::parse(link.trim()).ok()?;
    if u.scheme() != "nxm" {
        return None;
    }
    let domain = u.host_str()?.to_ascii_lowercase();
    let segs: Vec<&str> = u.path_segments()?.filter(|p| !p.is_empty()).collect();
    let (mod_id, file_id) = match segs.as_slice() {
        ["mods", m, "files", f, ..] => (m.to_string(), f.to_string()),
        _ => return None,
    };
    if !mod_id.chars().all(|c| c.is_ascii_digit()) || !file_id.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let q: HashMap<String, String> = u.query_pairs().into_owned().collect();
    let key = match (q.get("key"), q.get("expires").and_then(|e| e.parse().ok())) {
        (Some(k), Some(expires)) if !k.is_empty() => Some(NxmKey { key: k.clone(), expires }),
        _ => None,
    };
    Some(NxmLink { domain, mod_id, file_id, key })
}

/// `https://www.nexusmods.com/<game>/mods/<id>` (optionally `?tab=files&file_id=<file>`).
pub fn parse_page(input: &str) -> Option<(String, String, Option<String>)> {
    let u = url::Url::parse(input.trim()).ok()?;
    if !u.host_str()?.ends_with("nexusmods.com") {
        return None;
    }
    let segs: Vec<&str> = u.path_segments()?.filter(|p| !p.is_empty()).collect();
    let i = segs.iter().position(|p| *p == "mods")?;
    let domain = segs.get(i.checked_sub(1)?)?.to_ascii_lowercase();
    let mod_id = segs.get(i + 1)?.to_string();
    if !mod_id.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let file = u.query_pairs().find(|(k, _)| k == "file_id").map(|(_, v)| v.into_owned());
    Some((domain, mod_id, file))
}

/// Where a free account clicks "Slow download" for exactly this file; `nmm=1` starts the
/// mod manager download.
pub fn file_page(domain: &str, mod_id: &str, file_id: &str) -> String {
    format!("https://www.nexusmods.com/{domain}/mods/{mod_id}?tab=files&file_id={file_id}&nmm=1")
}

fn get(ctx: &Ctx<'_>, path: &str) -> Result<reqwest::RequestBuilder, String> {
    let key = ctx.settings.nexus_api_key.trim();
    if key.is_empty() {
        return Err("Add your Nexus Mods API key in Settings → Nexus Mods first".into());
    }
    Ok(ctx
        .http
        .get(format!("{API}{path}"))
        .header("apikey", key)
        .header("Application-Name", "Mods Manager")
        .header("Application-Version", env!("CARGO_PKG_VERSION")))
}

async fn json(req: reqwest::RequestBuilder, what: &str) -> Result<(reqwest::StatusCode, Value), String> {
    let resp = req.send().await.map_err(|e| format!("{what}: {}", e.without_url()))?;
    let status = resp.status();
    if status == reqwest::StatusCode::FORBIDDEN {
        return Ok((status, Value::Null));
    }
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Err("Nexus Mods doesn't accept the API key; check it in Settings → Nexus Mods".into());
    }
    let resp = check(resp, what).await?;
    Ok((status, resp.json().await.map_err(|e| format!("{what}: {}", e.without_url()))?))
}

/// The game's name on Nexus (`bladeandsorcery`) from that name or any nexusmods.com link to the
/// game or one of its mods: `nexusmods.com/games/bladeandsorcery`, `nexusmods.com/bladeandsorcery/mods/528`.
pub fn game_domain(input: &str) -> String {
    let s = input.trim().trim_end_matches('/');
    let with_scheme = if s.contains("nexusmods.com") && !s.contains("://") { format!("https://{s}") } else { s.to_string() };
    if let Ok(u) = url::Url::parse(&with_scheme) {
        if u.host_str().is_some_and(|h| h.ends_with("nexusmods.com")) {
            let segs: Vec<&str> = u.path_segments().map(|p| p.filter(|x| !x.is_empty()).collect()).unwrap_or_default();
            let name = match segs.as_slice() {
                ["games", name, ..] => *name,
                [name, ..] => *name,
                [] => "",
            };
            return name.to_ascii_lowercase();
        }
    }
    s.to_ascii_lowercase()
}

fn domain(ctx: &Ctx<'_>) -> Result<String, String> {
    // games saved with a full link before links were understood still work
    let d = game_domain(&ctx.game.data.nexus);
    if d.is_empty() {
        return Err(format!("{} isn't set up for Nexus Mods (Edit game → Nexus Mods)", ctx.game.data.name));
    }
    Ok(d)
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub name: String,
    pub premium: bool,
}

/// Who the API key belongs to, and whether downloads can skip the website.
pub async fn account(http: &reqwest::Client, api_key: &str) -> Result<Account, String> {
    let resp = http
        .get(format!("{API}/users/validate.json"))
        .header("apikey", api_key.trim())
        .header("Application-Name", "Mods Manager")
        .header("Application-Version", env!("CARGO_PKG_VERSION"))
        .send()
        .await
        .map_err(|e| format!("Nexus Mods: {}", e.without_url()))?;
    let v: Value = check(resp, "Nexus Mods").await?.json().await.map_err(|e| format!("Nexus Mods: {}", e.without_url()))?;
    Ok(Account { name: s(&v["name"]), premium: v["is_premium"].as_bool().unwrap_or(false) })
}

async fn files(ctx: &Ctx<'_>, mod_id: &str) -> Result<Vec<Value>, String> {
    let d = domain(ctx)?;
    let (_, v) = json(get(ctx, &format!("/games/{d}/mods/{mod_id}/files.json"))?, "Nexus Mods").await?;
    Ok(v["files"].as_array().cloned().unwrap_or_default())
}

/// The file a pin follows: the newest main file, preferring one with the same name
/// ("Main File", "Quest version"…) when a mod offers several.
fn newest<'a>(files: &'a [Value], same_name_as: Option<&str>) -> Option<&'a Value> {
    let main: Vec<&Value> = files.iter().filter(|f| f["category_name"] == "MAIN").collect();
    let pool: Vec<&Value> = if main.is_empty() {
        files.iter().filter(|f| !matches!(f["category_name"].as_str(), Some("ARCHIVED") | Some("OLD_VERSION") | Some("DELETED"))).collect()
    } else {
        main
    };
    let named: Vec<&Value> = same_name_as.map(|n| pool.iter().copied().filter(|f| s(&f["name"]) == n).collect()).unwrap_or_default();
    let pick = if named.is_empty() { pool } else { named };
    pick.into_iter().max_by_key(|f| f["uploaded_timestamp"].as_i64().unwrap_or(0))
}

fn to_new(ctx: &Ctx<'_>, m: &Value, f: &Value, mod_id: &str, target: &str) -> NewMod {
    let version = s(&f["version"]);
    NewMod {
        source: "nexus".into(),
        source_id: mod_id.to_string(),
        target: if target == "code" && ctx.game.data.loader != "none" { "code".into() } else { "content".into() },
        name: s(&m["name"]),
        version: if version.is_empty() { s(&f["name"]) } else { version },
        file_ref: s(&f["file_id"]),
        file_hash: String::new(),
        file_size: f["size_in_bytes"].as_i64().or_else(|| f["size_kb"].as_i64().map(|k| k * 1024)).unwrap_or(0),
        page_url: format!("https://www.nexusmods.com/{}/mods/{mod_id}", game_domain(&ctx.game.data.nexus)),
        thumb_url: s(&m["picture_url"]),
        summary: s(&m["summary"]),
        author: s(&m["author"]),
        extra: serde_json::json!({ "fileName": s(&f["name"]) }),
    }
}

pub async fn resolve(ctx: &Ctx<'_>, input: &str, target: &str) -> Result<NewMod, String> {
    let (page_domain, mod_id, file_id) = parse_page(input).ok_or("That doesn't look like a Nexus Mods page (nexusmods.com/<game>/mods/<id>)")?;
    let d = domain(ctx)?;
    if page_domain != d {
        return Err(format!("That mod is for \"{page_domain}\" on Nexus, but {} is set up as \"{d}\" (Edit game → Nexus Mods game)", ctx.game.data.name));
    }
    let (status, m) = json(get(ctx, &format!("/games/{d}/mods/{mod_id}.json"))?, "Nexus Mods").await?;
    if status == reqwest::StatusCode::FORBIDDEN {
        return Err("Nexus Mods refused the API key".into());
    }
    let all = files(ctx, &mod_id).await?;
    let f = match &file_id {
        Some(id) => all.iter().find(|f| s(&f["file_id"]) == *id).ok_or("That file isn't on the mod's Files tab anymore")?,
        None => newest(&all, None).ok_or("This mod has no files to download")?,
    };
    Ok(to_new(ctx, &m, f, &mod_id, target))
}

pub async fn latest(ctx: &Ctx<'_>, cur: &NewMod) -> Result<Option<NewMod>, String> {
    let all = files(ctx, &cur.source_id).await?;
    let name = cur.extra["fileName"].as_str();
    let Some(f) = newest(&all, name) else { return Ok(None) };
    if s(&f["file_id"]) == cur.file_ref {
        return Ok(None);
    }
    let d = domain(ctx)?;
    let (_, m) = json(get(ctx, &format!("/games/{d}/mods/{}.json", cur.source_id))?, "Nexus Mods").await?;
    Ok(Some(to_new(ctx, &m, f, &cur.source_id, &cur.target)))
}

/// The download, if the account may download it right now: Premium always, free accounts once
/// they clicked on the website (the `nxm://` key). Otherwise a `NEEDS_CLICK` error with the page.
pub async fn fetch(ctx: &Ctx<'_>, cur: &NewMod) -> Result<Fetch, String> {
    let d = domain(ctx)?;
    if !ctx.settings.nexus_logged_in {
        return Err(format!("{NEEDS_CLICK}{}", file_page(&d, &cur.source_id, &cur.file_ref)));
    }
    let slot = format!("{}:{}", cur.source_id, cur.file_ref);
    let now = chrono::Utc::now().timestamp();
    let key = ctx.nxm.lock().unwrap().get(&slot).filter(|k| k.expires > now).cloned();
    let mut req = get(ctx, &format!("/games/{d}/mods/{}/files/{}/download_link.json", cur.source_id, cur.file_ref))?;
    if let Some(k) = &key {
        req = req.query(&[("key", k.key.as_str()), ("expires", &k.expires.to_string())]);
    }
    let (status, links) = json(req, "Nexus Mods").await?;
    if status == reqwest::StatusCode::FORBIDDEN {
        if key.is_some() {
            ctx.nxm.lock().unwrap().remove(&slot);
        }
        return Err(format!("{NEEDS_CLICK}{}", file_page(&d, &cur.source_id, &cur.file_ref)));
    }
    let uri = links.as_array().and_then(|a| a.first()).map(|l| s(&l["URI"])).filter(|u| !u.is_empty()).ok_or("Nexus Mods gave no download link")?;
    // a key works once
    ctx.nxm.lock().unwrap().remove(&slot);
    let file_name = url::Url::parse(&uri)
        .ok()
        .and_then(|u| u.path_segments()?.filter(|p| !p.is_empty()).last().map(|n| n.to_string()))
        .map(|n| percent_decode(&n))
        .unwrap_or_else(|| format!("nexus-{}.zip", cur.file_ref));
    Ok(Fetch { request: ctx.http.get(uri), file_name })
}

fn percent_decode(s: &str) -> String {
    url::form_urlencoded::parse(format!("x={}", s.replace('+', "%2B")).as_bytes()).next().map(|(_, v)| v.into_owned()).unwrap_or_else(|| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_nxm_links() {
        let l = parse_nxm("nxm://bonelab/mods/123/files/4567?key=abc_DEF&expires=1794000000&user_id=99").unwrap();
        assert_eq!((l.domain.as_str(), l.mod_id.as_str(), l.file_id.as_str()), ("bonelab", "123", "4567"));
        assert_eq!(l.key, Some(NxmKey { key: "abc_DEF".into(), expires: 1794000000 }));
        assert!(parse_nxm("nxm://bonelab/mods/123/files/4567").unwrap().key.is_none());
        assert!(parse_nxm("https://example.com/mods/1/files/2").is_none());
        assert!(parse_nxm("nxm://bonelab/collections/x").is_none());
    }

    #[test]
    fn reads_the_game_from_any_nexus_link() {
        for input in [
            "bladeandsorcery",
            "BladeAndSorcery ",
            "https://www.nexusmods.com/games/bladeandsorcery",
            "https://www.nexusmods.com/games/bladeandsorcery/",
            "https://www.nexusmods.com/bladeandsorcery/mods/528",
            "nexusmods.com/bladeandsorcery",
        ] {
            assert_eq!(game_domain(input), "bladeandsorcery", "{input}");
        }
        assert_eq!(game_domain(""), "");
    }

    #[test]
    fn reads_mod_pages() {
        assert_eq!(parse_page("https://www.nexusmods.com/bladeandsorcery/mods/8001"), Some(("bladeandsorcery".into(), "8001".into(), None)));
        assert_eq!(
            parse_page("https://www.nexusmods.com/bonelab/mods/12?tab=files&file_id=345"),
            Some(("bonelab".into(), "12".into(), Some("345".into())))
        );
        assert_eq!(parse_page("https://mod.io/g/bonelab/m/x"), None);
    }

    #[test]
    fn follows_the_newest_main_file_with_the_same_name() {
        let files = serde_json::json!([
            { "file_id": 1, "name": "Main File", "category_name": "OLD_VERSION", "uploaded_timestamp": 1 },
            { "file_id": 2, "name": "Main File", "category_name": "MAIN", "uploaded_timestamp": 5 },
            { "file_id": 3, "name": "Quest version", "category_name": "MAIN", "uploaded_timestamp": 9 },
        ]);
        let files = files.as_array().unwrap();
        assert_eq!(s(&newest(files, Some("Main File")).unwrap()["file_id"]), "2");
        assert_eq!(s(&newest(files, None).unwrap()["file_id"]), "3");
    }
}
