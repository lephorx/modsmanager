//! Anything else: a direct download link. Links can change what they serve,
//! so the file's SHA-256 is the pin.

use super::{AddOptions, Ctx, Fetch};
use crate::db::NewMod;
use crate::install;

fn file_name_of(link: &str) -> String {
    url::Url::parse(link)
        .ok()
        .and_then(|u| u.path_segments()?.filter(|p| !p.is_empty()).last().map(str::to_string))
        .unwrap_or_else(|| "download".into())
}

/// Downloads the file once to pin its hash.
async fn pin(ctx: &Ctx<'_>, link: &str) -> Result<install::Downloaded, String> {
    let fetch = Fetch { request: ctx.http.get(link), file_name: file_name_of(link), md5: None, sha256: None };
    let dest = ctx.temp.join(format!("pin-{}-{}", std::process::id(), chrono::Utc::now().timestamp_millis()));
    let got = install::download(fetch, &dest, |_, _| {}, || false).await;
    let _ = tokio::fs::remove_file(&dest).await;
    got
}

pub async fn resolve(ctx: &Ctx<'_>, link: &str, opts: &AddOptions) -> Result<NewMod, String> {
    let got = pin(ctx, link).await?;
    let stem = got.file_name.rsplit_once('.').map(|(a, _)| a.to_string()).unwrap_or_else(|| got.file_name.clone());
    Ok(NewMod {
        source: "link".into(),
        source_id: link.to_string(),
        target: if opts.target == "code" { "code".into() } else { "content".into() },
        name: if opts.name.trim().is_empty() { stem } else { opts.name.trim().to_string() },
        version: got.sha256[..8].to_string(),
        file_ref: String::new(),
        file_hash: got.sha256,
        file_size: got.size as i64,
        page_url: link.to_string(),
        thumb_url: String::new(),
        summary: got.file_name,
        author: String::new(),
        extra: serde_json::json!({}),
    })
}

pub async fn latest(ctx: &Ctx<'_>, cur: &NewMod) -> Result<Option<NewMod>, String> {
    let got = pin(ctx, &cur.source_id).await?;
    if got.sha256 == cur.file_hash {
        return Ok(None);
    }
    let mut m = cur.clone();
    m.version = got.sha256[..8].to_string();
    m.file_hash = got.sha256;
    m.file_size = got.size as i64;
    Ok(Some(m))
}

pub fn fetch(ctx: &Ctx<'_>, cur: &NewMod) -> Fetch {
    Fetch {
        request: ctx.http.get(&cur.source_id),
        file_name: file_name_of(&cur.source_id),
        md5: None,
        sha256: Some(cur.file_hash.clone()).filter(|h| !h.is_empty()),
    }
}
