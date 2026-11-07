//! Downloading, checking and placing mod files, and taking them out again.
//!
//! Content mods (mod.io pallets) go into BONELAB's Mods folder as top-level folders.
//! Code mods (MelonLoader) go into the game folder using MelonLoader's layout:
//! `Mods/`, `Plugins/`, `UserLibs/`, `UserData/`; a loose DLL goes into `Mods/`.

use std::io::Read;
use std::path::{Component, Path, PathBuf};

use futures_util::StreamExt;
use md5::{Digest as _, Md5};
use serde::Serialize;
use sha2::Sha256;
use tokio::io::AsyncWriteExt;

use crate::config::{Installed, Local, Root, Settings};
use crate::sources::{check, Fetch};

/// MelonLoader's folders in the game directory, spelled the way MelonLoader does.
pub const MELON_ROOTS: [&str; 4] = ["Mods", "Plugins", "UserLibs", "UserData"];

pub struct Downloaded {
    pub file_name: String,
    pub size: u64,
    pub sha256: String,
}

fn disposition_name(resp: &reqwest::Response) -> Option<String> {
    let cd = resp.headers().get(reqwest::header::CONTENT_DISPOSITION)?.to_str().ok()?;
    let raw = if let Some(i) = cd.find("filename*=") {
        let v = &cd[i + 10..];
        let v = v.split(';').next()?.trim();
        v.rsplit("''").next()?.to_string()
    } else {
        let i = cd.find("filename=")?;
        cd[i + 9..].split(';').next()?.trim().trim_matches('"').trim_matches('\'').to_string()
    };
    let name = raw.rsplit(['/', '\\']).next()?.to_string();
    (!name.is_empty()).then_some(name)
}

/// Streams the file to `dest`, reporting `(done, total)`, and checks it against the expected hash.
pub async fn download(fetch: Fetch, dest: &Path, mut progress: impl FnMut(u64, u64)) -> Result<Downloaded, String> {
    let resp = fetch.request.send().await.map_err(|e| format!("Download failed: {e}"))?;
    let resp = check(resp, "Download failed").await?;
    let total = resp.content_length().unwrap_or(0);
    let file_name = disposition_name(&resp).unwrap_or(fetch.file_name);
    if let Some(dir) = dest.parent() {
        tokio::fs::create_dir_all(dir).await.map_err(|e| e.to_string())?;
    }
    let mut file = tokio::fs::File::create(dest).await.map_err(|e| e.to_string())?;
    let (mut md5, mut sha) = (Md5::new(), Sha256::new());
    let mut done = 0u64;
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Download interrupted: {e}"))?;
        md5.update(&chunk);
        sha.update(&chunk);
        file.write_all(&chunk).await.map_err(|e| e.to_string())?;
        done += chunk.len() as u64;
        progress(done, total.max(done));
    }
    file.flush().await.map_err(|e| e.to_string())?;
    drop(file);
    let md5 = hex::encode(md5.finalize());
    let sha256 = hex::encode(sha.finalize());
    if let Some(want) = fetch.md5 {
        if !want.eq_ignore_ascii_case(&md5) {
            return Err("The download doesn't match the pinned file (MD5 differs). Try again.".into());
        }
    }
    if let Some(want) = fetch.sha256 {
        if !want.eq_ignore_ascii_case(&sha256) {
            return Err("The link now serves a different file than the one on the list. Use \"Update\" to pin the new one.".into());
        }
    }
    Ok(Downloaded { file_name, size: done, sha256 })
}

fn is_zip(path: &Path) -> bool {
    let mut magic = [0u8; 4];
    std::fs::File::open(path).and_then(|mut f| f.read_exact(&mut magic)).is_ok() && magic == *b"PK\x03\x04"
}

/// Unpacks the download into `staging` (or copies it there when it isn't a zip).
pub fn unpack(file: &Path, file_name: &str, staging: &Path) -> Result<(), String> {
    if staging.exists() {
        std::fs::remove_dir_all(staging).map_err(|e| e.to_string())?;
    }
    std::fs::create_dir_all(staging).map_err(|e| e.to_string())?;
    if !is_zip(file) {
        let name = sanitize(file_name);
        std::fs::copy(file, staging.join(if name.is_empty() { "download".into() } else { name })).map_err(|e| e.to_string())?;
        return Ok(());
    }
    let mut zip = zip::ZipArchive::new(std::fs::File::open(file).map_err(|e| e.to_string())?).map_err(|e| format!("Broken zip: {e}"))?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| format!("Broken zip: {e}"))?;
        // never write outside the staging folder
        let Some(rel) = entry.enclosed_name() else { continue };
        let out = staging.join(rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
            continue;
        }
        if let Some(dir) = out.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let mut f = std::fs::File::create(&out).map_err(|e| e.to_string())?;
        std::io::copy(&mut entry, &mut f).map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_alphanumeric() || "-_. ()".contains(c) { c } else { '_' })
        .collect::<String>()
        .trim_matches(['.', ' '])
        .to_string()
}

fn entries(dir: &Path) -> Result<Vec<std::fs::DirEntry>, String> {
    let mut v: Vec<_> = std::fs::read_dir(dir).map_err(|e| e.to_string())?.filter_map(Result::ok).collect();
    v.sort_by_key(|e| e.file_name());
    Ok(v)
}

fn remove_any(path: &Path) -> std::io::Result<()> {
    if path.is_dir() {
        std::fs::remove_dir_all(path)
    } else if path.exists() {
        std::fs::remove_file(path)
    } else {
        Ok(())
    }
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let dest = to.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_dir(&e.path(), &dest)?;
        } else {
            std::fs::copy(e.path(), dest)?;
        }
    }
    Ok(())
}

/// Rename when possible, copy across drives otherwise. Replaces what's at `to`.
fn move_path(from: &Path, to: &Path) -> Result<(), String> {
    remove_any(to).map_err(|e| format!("Can't replace {}: {e}", to.display()))?;
    if let Some(dir) = to.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    if from.is_dir() {
        copy_dir(from, to).map_err(|e| e.to_string())?;
        std::fs::remove_dir_all(from).map_err(|e| e.to_string())
    } else {
        std::fs::copy(from, to).map_err(|e| e.to_string())?;
        std::fs::remove_file(from).map_err(|e| e.to_string())
    }
}

/// Content mods: the archive's top-level folders become folders in the Mods folder.
/// Loose files get wrapped in a folder named after the mod.
pub fn place_content(staging: &Path, mods_dir: &Path, fallback: &str) -> Result<Vec<String>, String> {
    std::fs::create_dir_all(mods_dir).map_err(|e| format!("Can't create the Mods folder: {e}"))?;
    let items = entries(staging)?;
    if !items.is_empty() && items.iter().all(|e| e.path().is_dir()) {
        let mut paths = Vec::new();
        for e in items {
            let name = e.file_name().to_string_lossy().into_owned();
            move_path(&e.path(), &mods_dir.join(&name))?;
            paths.push(name);
        }
        return Ok(paths);
    }
    let mut folder = sanitize(fallback);
    if folder.is_empty() {
        folder = "Mod".into();
    }
    move_path(staging, &mods_dir.join(&folder))?;
    Ok(vec![folder])
}

fn melon_root(name: &str) -> Option<&'static str> {
    MELON_ROOTS.iter().copied().find(|r| r.eq_ignore_ascii_case(name))
}

fn walk(dir: &Path, base: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for e in std::fs::read_dir(dir)? {
        let e = e?;
        if e.file_type()?.is_dir() {
            walk(&e.path(), base, out)?;
        } else if let Ok(rel) = e.path().strip_prefix(base) {
            out.push(rel.to_path_buf());
        }
    }
    Ok(())
}

/// Code mods: files keep MelonLoader's layout inside the game folder. Returns the files placed,
/// relative to the game folder with `/` separators.
pub fn place_code(staging: &Path, game_dir: &Path) -> Result<Vec<String>, String> {
    // packages that wrap everything in one folder: look inside it
    let mut base = staging.to_path_buf();
    loop {
        let items = entries(&base)?;
        match items.as_slice() {
            [only] if only.path().is_dir() && melon_root(&only.file_name().to_string_lossy()).is_none() => base = only.path(),
            _ => break,
        }
    }
    let mut files = Vec::new();
    walk(&base, &base, &mut files).map_err(|e| e.to_string())?;
    let mut placed = Vec::new();
    for rel in files {
        let parts: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
        let Some(first) = parts.first() else { continue };
        let file_name = parts.last().cloned().unwrap_or_default();
        let is_dll = file_name.to_ascii_lowercase().ends_with(".dll");
        // loose files at the top: DLLs are mods; manifest.json, icon.png, README and the like are for the website
        let dest: Option<Vec<String>> = if parts.len() == 1 {
            is_dll.then(|| vec!["Mods".into(), file_name.clone()])
        } else if let Some(root) = melon_root(first) {
            let mut v = vec![root.to_string()];
            v.extend(parts[1..].iter().cloned());
            Some(v)
        } else if is_dll {
            Some(vec!["Mods".into(), file_name.clone()])
        } else {
            None
        };
        let Some(dest) = dest else { continue };
        let target = dest.iter().fold(game_dir.to_path_buf(), |p, s| p.join(s));
        move_path(&base.join(&rel), &target)?;
        placed.push(dest.join("/"));
    }
    if placed.is_empty() {
        return Err("This package has no DLLs or MelonLoader folders to install".into());
    }
    Ok(placed)
}

/// A stored path that stays inside its root.
pub fn safe_join(root: &Path, rel: &str) -> Option<PathBuf> {
    let p = Path::new(rel);
    if rel.is_empty() || p.components().any(|c| !matches!(c, Component::Normal(_))) {
        return None;
    }
    Some(root.join(p))
}

pub fn root_dir(settings: &Settings, root: Root) -> PathBuf {
    match root {
        Root::Mods => PathBuf::from(&settings.mods_dir),
        Root::Game => PathBuf::from(&settings.game_dir),
    }
}

/// Every file or folder of the install is still there.
pub fn is_present(settings: &Settings, inst: &Installed) -> bool {
    let root = root_dir(settings, inst.root);
    !inst.paths.is_empty() && inst.paths.iter().all(|p| safe_join(&root, p).is_some_and(|p| p.exists()))
}

/// Deletes an install's files; for code mods also the folders that became empty
/// (but never MelonLoader's own folders).
pub fn remove(settings: &Settings, inst: &Installed) -> Result<(), String> {
    let root = root_dir(settings, inst.root);
    if root.as_os_str().is_empty() {
        return Err("Set the folder in Settings first".into());
    }
    for rel in &inst.paths {
        let Some(path) = safe_join(&root, rel) else { continue };
        remove_any(&path).map_err(|e| format!("Can't delete {}: {e}", path.display()))?;
        if inst.root == Root::Game {
            let mut dir = path.parent().map(Path::to_path_buf);
            while let Some(d) = dir {
                let is_melon_root = d.parent() == Some(root.as_path());
                if d == root || is_melon_root || std::fs::remove_dir(&d).is_err() {
                    break;
                }
                dir = d.parent().map(Path::to_path_buf);
            }
        }
    }
    Ok(())
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Unmanaged {
    /// `mods/<folder>` or `game/Mods/<file>.dll`
    pub key: String,
    pub name: String,
    pub path: String,
    pub is_dir: bool,
}

/// What's in the mod folders that no mod on the list put there.
pub fn unmanaged(settings: &Settings, local: &Local) -> Vec<Unmanaged> {
    let mut out = Vec::new();
    let owned = |root: Root| -> std::collections::HashSet<String> {
        local
            .installed
            .values()
            .filter(|i| i.root == root)
            .flat_map(|i| i.paths.iter().map(|p| p.to_ascii_lowercase()))
            .collect()
    };
    if !settings.mods_dir.is_empty() {
        let owned = owned(Root::Mods);
        if let Ok(items) = entries(Path::new(&settings.mods_dir)) {
            for e in items {
                let name = e.file_name().to_string_lossy().into_owned();
                let key = format!("mods/{name}");
                if name.starts_with('.') || owned.contains(&name.to_ascii_lowercase()) || local.ignored.contains(&key) {
                    continue;
                }
                out.push(Unmanaged { key, name, path: e.path().to_string_lossy().into_owned(), is_dir: e.path().is_dir() });
            }
        }
    }
    if !settings.game_dir.is_empty() {
        let owned = owned(Root::Game);
        for sub in ["Mods", "Plugins"] {
            let Ok(items) = entries(&Path::new(&settings.game_dir).join(sub)) else { continue };
            for e in items {
                let name = e.file_name().to_string_lossy().into_owned();
                if !name.to_ascii_lowercase().ends_with(".dll") {
                    continue;
                }
                let rel = format!("{sub}/{name}");
                let key = format!("game/{rel}");
                if owned.contains(&rel.to_ascii_lowercase()) || local.ignored.contains(&key) {
                    continue;
                }
                out.push(Unmanaged { key, name: rel, path: e.path().to_string_lossy().into_owned(), is_dir: false });
            }
        }
    }
    out
}

pub fn delete_unmanaged(settings: &Settings, key: &str) -> Result<(), String> {
    let (root, rel) = if let Some(rel) = key.strip_prefix("mods/") {
        (PathBuf::from(&settings.mods_dir), rel)
    } else if let Some(rel) = key.strip_prefix("game/") {
        (PathBuf::from(&settings.game_dir), rel)
    } else {
        return Err("Unknown file".into());
    };
    let path = safe_join(&root, rel).ok_or("Unknown file")?;
    remove_any(&path).map_err(|e| format!("Can't delete {}: {e}", path.display()))
}

/// MelonLoader is set up in the game folder.
pub fn melon_installed(settings: &Settings) -> bool {
    !settings.game_dir.is_empty() && Path::new(&settings.game_dir).join("MelonLoader").is_dir()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("mm-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn code_mods_follow_melonloader_layout() {
        let staging = tmp("code-staging");
        let game = tmp("code-game");
        std::fs::create_dir_all(staging.join("Mods")).unwrap();
        std::fs::write(staging.join("Mods/Foo.dll"), b"x").unwrap();
        std::fs::create_dir_all(staging.join("userlibs")).unwrap();
        std::fs::write(staging.join("userlibs/Lib.dll"), b"x").unwrap();
        std::fs::write(staging.join("Loose.dll"), b"x").unwrap();
        std::fs::write(staging.join("manifest.json"), b"{}").unwrap();
        let mut placed = place_code(&staging, &game).unwrap();
        placed.sort();
        assert_eq!(placed, vec!["Mods/Foo.dll", "Mods/Loose.dll", "UserLibs/Lib.dll"]);
        assert!(game.join("Mods/Foo.dll").is_file());
        assert!(!game.join("manifest.json").exists());

        let settings = Settings { game_dir: game.to_string_lossy().into(), ..Default::default() };
        let inst = Installed { name: "Foo".into(), pin: String::new(), version: String::new(), root: Root::Game, paths: placed, installed_at: chrono::Utc::now() };
        assert!(is_present(&settings, &inst));
        remove(&settings, &inst).unwrap();
        assert!(!game.join("Mods/Foo.dll").exists());
        assert!(game.join("Mods").is_dir(), "MelonLoader's own folders stay");
    }

    #[test]
    fn content_folders_are_moved_and_loose_files_wrapped() {
        let staging = tmp("content-staging");
        let mods = tmp("content-mods");
        std::fs::create_dir_all(staging.join("Author.Pallet")).unwrap();
        std::fs::write(staging.join("Author.Pallet/pallet.json"), b"{}").unwrap();
        assert_eq!(place_content(&staging, &mods, "x").unwrap(), vec!["Author.Pallet"]);
        assert!(mods.join("Author.Pallet/pallet.json").is_file());

        let staging = tmp("content-staging2");
        std::fs::write(staging.join("thing.bin"), b"x").unwrap();
        assert_eq!(place_content(&staging, &mods, "My Mod").unwrap(), vec!["My Mod"]);
        assert!(mods.join("My Mod/thing.bin").is_file());
    }

    #[test]
    fn stored_paths_cannot_escape() {
        assert!(safe_join(Path::new("C:/x"), "../evil").is_none());
        assert!(safe_join(Path::new("C:/x"), "").is_none());
        assert!(safe_join(Path::new("C:/x"), "Mods/a.dll").is_some());
    }
}
