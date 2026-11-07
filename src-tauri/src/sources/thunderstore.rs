//! Thunderstore: MelonLoader code mods (DLLs). Versions there never change, so the version is the pin.

use serde_json::Value;

use super::{check, s, Ctx, Fetch};
use crate::db::NewMod;

const BASE: &str = "https://thunderstore.io";

/// `https://thunderstore.io/c/bonelab/p/Ns/Name/` (optionally `/v/1.2.3/`), or `Ns-Name[-1.2.3]`.
pub fn parse(input: &str) -> Option<(String, String, Option<String>)> {
    let input = input.trim();
    if let Ok(u) = url::Url::parse(input) {
        let segs: Vec<String> = u.path_segments()?.filter(|p| !p.is_empty()).map(str::to_string).collect();
        let i = segs.iter().position(|p| p == "p" || p == "package")?;
        let ns = segs.get(i + 1)?.clone();
        let name = segs.get(i + 2)?.clone();
        let ver = if segs.get(i + 3).map(String::as_str) == Some("v") { segs.get(i + 4).cloned() } else { None };
        return Some((ns, name, ver));
    }
    let parts: Vec<&str> = input.split('-').collect();
    if parts.len() < 2 {
        return None;
    }
    let last = parts[parts.len() - 1];
    let is_version = last.split('.').count() == 3 && last.split('.').all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()));
    if is_version && parts.len() >= 3 {
        Some((parts[0].into(), parts[1..parts.len() - 1].join("-"), Some(last.into())))
    } else {
        Some((parts[0].into(), parts[1..].join("-"), None))
    }
}

/// MelonLoader itself is installed by its own installer, not as a mod.
pub fn is_loader(ns: &str, name: &str) -> bool {
    ns.eq_ignore_ascii_case("LavaGang") && name.eq_ignore_ascii_case("MelonLoader")
}

async fn json(ctx: &Ctx<'_>, url: String) -> Result<Value, String> {
    let resp = ctx.http.get(url).send().await.map_err(|e| format!("Thunderstore: {e}"))?;
    check(resp, "Thunderstore").await?.json().await.map_err(|e| format!("Thunderstore: {e}"))
}

async fn package(ctx: &Ctx<'_>, ns: &str, name: &str) -> Result<Value, String> {
    json(ctx, format!("{BASE}/api/experimental/package/{ns}/{name}/")).await
}

fn to_new(pkg: &Value, ver: &Value) -> NewMod {
    let ns = s(&pkg["namespace"]);
    let name = s(&pkg["name"]);
    NewMod {
        source: "thunderstore".into(),
        source_id: format!("{ns}/{name}"),
        target: "code".into(),
        name: name.replace('_', " "),
        version: s(&ver["version_number"]),
        file_ref: s(&ver["version_number"]),
        file_hash: String::new(),
        file_size: ver["file_size"].as_i64().unwrap_or(0),
        page_url: s(&pkg["package_url"]),
        thumb_url: s(&ver["icon"]),
        summary: s(&ver["description"]),
        author: s(&pkg["owner"]),
        extra: serde_json::json!({}),
    }
}

fn deps(ver: &Value) -> Vec<String> {
    ver["dependencies"].as_array().map(|a| a.iter().map(s).collect()).unwrap_or_default()
}

/// The given version (or the newest) and its dependency strings.
pub async fn pinned(ctx: &Ctx<'_>, ns: &str, name: &str, version: Option<&str>) -> Result<(NewMod, Vec<String>), String> {
    let pkg = package(ctx, ns, name).await?;
    let ver = match version {
        Some(v) if v != s(&pkg["latest"]["version_number"]) => {
            json(ctx, format!("{BASE}/api/experimental/package/{ns}/{name}/{v}/")).await?
        }
        _ => pkg["latest"].clone(),
    };
    Ok((to_new(&pkg, &ver), deps(&ver)))
}

pub async fn resolve(ctx: &Ctx<'_>, input: &str) -> Result<(NewMod, Vec<String>), String> {
    let (ns, name, ver) = parse(input).ok_or("That doesn't look like a Thunderstore package link")?;
    if is_loader(&ns, &name) {
        return Err("MelonLoader is installed with its own installer, not as a mod".into());
    }
    pinned(ctx, &ns, &name, ver.as_deref()).await
}

pub async fn latest(ctx: &Ctx<'_>, cur: &NewMod) -> Result<Option<NewMod>, String> {
    let (ns, name) = cur.source_id.split_once('/').ok_or("Bad Thunderstore id")?;
    let pkg = package(ctx, ns, name).await?;
    let ver = &pkg["latest"];
    Ok((s(&ver["version_number"]) != cur.file_ref).then(|| to_new(&pkg, ver)))
}

pub fn fetch(ctx: &Ctx<'_>, cur: &NewMod) -> Fetch {
    let (ns, name) = cur.source_id.split_once('/').unwrap_or(("", ""));
    Fetch {
        request: ctx.http.get(format!("{BASE}/package/download/{ns}/{name}/{}/", cur.file_ref)),
        file_name: format!("{ns}-{name}-{}.zip", cur.file_ref),
        md5: None,
        sha256: None,
    }
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn parses_links_and_dependency_strings() {
        assert_eq!(
            parse("https://thunderstore.io/c/bonelab/p/Lakatrazz/Fusion/"),
            Some(("Lakatrazz".into(), "Fusion".into(), None))
        );
        assert_eq!(
            parse("https://thunderstore.io/c/bonelab/p/Lakatrazz/Fusion/v/1.9.3/"),
            Some(("Lakatrazz".into(), "Fusion".into(), Some("1.9.3".into())))
        );
        assert_eq!(parse("LavaGang-MelonLoader-0.6.1"), Some(("LavaGang".into(), "MelonLoader".into(), Some("0.6.1".into()))));
        assert_eq!(parse("Ns-Some_Name"), Some(("Ns".into(), "Some_Name".into(), None)));
    }
}
