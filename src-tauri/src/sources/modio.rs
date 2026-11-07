//! mod.io: content mods (avatars, maps, items…) for any game on mod.io.

use serde_json::Value;

use super::{check, s, Ctx, Fetch, SearchHit};
use crate::db::NewMod;

const API: &str = "https://api.mod.io/v1";

fn get(ctx: &Ctx<'_>, path: &str) -> Result<reqwest::RequestBuilder, String> {
    let st = ctx.settings;
    let game = ctx.game.data.modio_id;
    if game <= 0 {
        return Err(format!("{} isn't set up for mod.io (Edit game → mod.io)", ctx.game.data.name));
    }
    let req = ctx.http.get(format!("{API}/games/{game}{path}"));
    auth(st, req)
}

fn auth(st: &crate::config::Settings, req: reqwest::RequestBuilder) -> Result<reqwest::RequestBuilder, String> {
    if !st.modio_token.trim().is_empty() {
        Ok(req.bearer_auth(st.modio_token.trim()))
    } else if !st.modio_api_key.trim().is_empty() {
        Ok(req.query(&[("api_key", st.modio_api_key.trim())]))
    } else {
        Err("Add your mod.io API key in Settings → mod.io first".into())
    }
}

async fn json(req: reqwest::RequestBuilder, what: &str) -> Result<Value, String> {
    // without_url: the URL carries the API key, which never goes into an error message
    let resp = req.send().await.map_err(|e| format!("{what}: {}", e.without_url()))?;
    check(resp, what).await?.json().await.map_err(|e| format!("{what}: {}", e.without_url()))
}

async fn find(ctx: &Ctx<'_>, input: &str) -> Result<Value, String> {
    let input = input.trim().split('#').next().unwrap_or("").trim_end_matches('/');
    if !input.is_empty() && input.chars().all(|c| c.is_ascii_digit()) {
        return json(get(ctx, &format!("/mods/{input}"))?, "mod.io").await;
    }
    // https://mod.io/g/bonelab/m/some-mod
    let name_id = url::Url::parse(input)
        .ok()
        .and_then(|u| {
            let segs: Vec<String> = u.path_segments()?.map(str::to_string).collect();
            let i = segs.iter().position(|p| p == "m")?;
            segs.get(i + 1).cloned()
        })
        .ok_or("That doesn't look like a mod.io mod link (…/g/<game>/m/<mod>)")?;
    let list = json(get(ctx, "/mods")?.query(&[("name_id", name_id.as_str())]), "mod.io").await?;
    list["data"].get(0).cloned().ok_or_else(|| format!("mod.io has no {} mod called \"{name_id}\"", ctx.game.data.name))
}

/// The file people get today: the Windows build when the mod has per-platform files.
fn live_file(m: &Value) -> Option<String> {
    m["platforms"]
        .as_array()
        .and_then(|ps| ps.iter().find(|p| p["platform"] == "windows"))
        .map(|p| s(&p["modfile_live"]))
        .filter(|id| !id.is_empty() && id != "0")
        .or_else(|| Some(s(&m["modfile"]["id"])).filter(|id| !id.is_empty() && id != "0"))
}

async fn file(ctx: &Ctx<'_>, mod_id: &str, file_id: &str) -> Result<Value, String> {
    json(get(ctx, &format!("/mods/{mod_id}/files/{file_id}"))?, "mod.io file").await
}

fn to_new(m: &Value, f: &Value) -> NewMod {
    let version = s(&f["version"]);
    NewMod {
        source: "modio".into(),
        source_id: s(&m["id"]),
        target: "content".into(),
        name: s(&m["name"]),
        version: if version.is_empty() { format!("file {}", s(&f["id"])) } else { version },
        file_ref: s(&f["id"]),
        file_hash: s(&f["filehash"]["md5"]),
        file_size: f["filesize"].as_i64().unwrap_or(0),
        page_url: s(&m["profile_url"]),
        thumb_url: s(&m["logo"]["thumb_320x180"]),
        summary: s(&m["summary"]),
        author: s(&m["submitted_by"]["username"]),
        extra: serde_json::json!({}),
    }
}

pub async fn resolve(ctx: &Ctx<'_>, input: &str) -> Result<NewMod, String> {
    let m = find(ctx, input).await?;
    let fid = live_file(&m).ok_or("This mod has no file for Windows yet")?;
    let f = file(ctx, &s(&m["id"]), &fid).await?;
    Ok(to_new(&m, &f))
}

pub async fn latest(ctx: &Ctx<'_>, cur: &NewMod) -> Result<Option<NewMod>, String> {
    let m = json(get(ctx, &format!("/mods/{}", cur.source_id))?, "mod.io").await?;
    let Some(fid) = live_file(&m) else { return Ok(None) };
    if fid == cur.file_ref {
        return Ok(None);
    }
    let f = file(ctx, &cur.source_id, &fid).await?;
    Ok(Some(to_new(&m, &f)))
}

pub async fn fetch(ctx: &Ctx<'_>, cur: &NewMod) -> Result<Fetch, String> {
    let f = file(ctx, &cur.source_id, &cur.file_ref).await?;
    let url = s(&f["download"]["binary_url"]);
    if url.is_empty() {
        return Err("mod.io didn't give a download link for this file".into());
    }
    let mut request = ctx.http.get(url);
    let token = ctx.settings.modio_token.trim();
    if !token.is_empty() {
        request = request.bearer_auth(token);
    }
    let md5 = Some(s(&f["filehash"]["md5"])).filter(|h| !h.is_empty());
    Ok(Fetch { request, file_name: s(&f["filename"]), md5, sha256: None })
}

pub async fn search(ctx: &Ctx<'_>, query: &str) -> Result<Vec<SearchHit>, String> {
    let mut req = get(ctx, "/mods")?.query(&[("_limit", "24"), ("_sort", "-popular")]);
    if !query.trim().is_empty() {
        req = req.query(&[("_q", query.trim())]);
    }
    let list = json(req, "mod.io search").await?;
    Ok(list["data"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|m| SearchHit {
                    id: s(&m["id"]),
                    input: s(&m["profile_url"]),
                    name: s(&m["name"]),
                    summary: s(&m["summary"]),
                    thumb: s(&m["logo"]["thumb_320x180"]),
                    author: s(&m["submitted_by"]["username"]),
                    downloads: m["stats"]["downloads_total"].as_i64().unwrap_or(0),
                })
                .collect()
        })
        .unwrap_or_default())
}

/// The mod.io game id for `mod.io/g/<slug>` (a slug, a game link or a number).
pub async fn game_id(http: &reqwest::Client, settings: &crate::config::Settings, input: &str) -> Result<(i64, String), String> {
    let input = input.trim().trim_end_matches('/');
    if let Ok(id) = input.parse::<i64>() {
        return Ok((id, String::new()));
    }
    let slug = url::Url::parse(input)
        .ok()
        .and_then(|u| {
            let segs: Vec<String> = u.path_segments()?.map(str::to_string).collect();
            let i = segs.iter().position(|p| p == "g")?;
            segs.get(i + 1).cloned()
        })
        .unwrap_or_else(|| input.to_string());
    let req = auth(settings, http.get(format!("{API}/games")).query(&[("name_id", slug.as_str())]))?;
    let list = json(req, "mod.io").await?;
    let g = list["data"].get(0).ok_or_else(|| format!("mod.io has no game called \"{slug}\""))?;
    Ok((g["id"].as_i64().unwrap_or(0), slug))
}
