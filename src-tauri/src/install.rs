//! Downloading, checking and placing mod files, and taking them out again.
//!
//! Content mods go into the game's content folder as top-level folders (BONELAB pallets,
//! Blade & Sorcery mod folders…). Code mods go into the game folder in the layout of the game's
//! loader: MelonLoader (`Mods/`, `Plugins/`, `UserLibs/`, `UserData/`) or BepInEx
//! (`BepInEx/plugins/<package>/…`, like r2modman).

use std::io::Read;
use std::path::{Component, Path, PathBuf};

use futures_util::StreamExt;
use sha2::Digest as _;
use serde::Serialize;
use sha2::Sha256;
use tokio::io::AsyncWriteExt;

use crate::config::{GamePaths, Installed, LocalGame, Root};
use crate::sources::{check, Fetch};

/// MelonLoader's folders in the game directory, spelled the way MelonLoader does.
pub const MELON_ROOTS: [&str; 4] = ["Mods", "Plugins", "UserLibs", "UserData"];
/// BepInEx's folders inside `BepInEx/`; the first three get a folder per package (as r2modman does).
const BEPINEX_PER_PACKAGE: [&str; 3] = ["plugins", "patchers", "monomod"];
const BEPINEX_SHARED: [&str; 2] = ["config", "core"];
/// Files a Thunderstore package carries for the website, not for the game.
const PACKAGE_META: [&str; 5] = ["manifest.json", "icon.png", "readme.md", "changelog.md", "license"];

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

/// What a download stopped by `cancelled` returns.
pub const CANCELLED: &str = "Cancelled";

/// Streams the file to `dest`, reporting `(done, total)`. A download shorter than the server
/// announced, or a zip that isn't whole, is an error (worth retrying).
/// Stops (with `CANCELLED`) as soon as `cancelled()` says so; the caller deletes the partial file.
pub async fn download(fetch: &Fetch, dest: &Path, mut progress: impl FnMut(u64, u64), cancelled: impl Fn() -> bool) -> Result<Downloaded, String> {
    if cancelled() {
        return Err(CANCELLED.into());
    }
    let request = fetch.request.try_clone().ok_or("Can't repeat this download request")?;
    let resp = request.send().await.map_err(|e| format!("Download failed: {}", e.without_url()))?;
    let resp = check(resp, "Download failed").await?;
    let total = resp.content_length().unwrap_or(0);
    let file_name = disposition_name(&resp).unwrap_or_else(|| fetch.file_name.clone());
    if let Some(dir) = dest.parent() {
        tokio::fs::create_dir_all(dir).await.map_err(|e| e.to_string())?;
    }
    let mut file = tokio::fs::File::create(dest).await.map_err(|e| e.to_string())?;
    let mut sha = Sha256::new();
    let mut done = 0u64;
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        if cancelled() {
            return Err(CANCELLED.into());
        }
        let chunk = chunk.map_err(|e| format!("Download interrupted: {}", e.without_url()))?;
        sha.update(&chunk);
        file.write_all(&chunk).await.map_err(|e| e.to_string())?;
        done += chunk.len() as u64;
        progress(done, total.max(done));
    }
    file.flush().await.map_err(|e| e.to_string())?;
    drop(file);
    if total > 0 && done < total {
        return Err(format!("Download interrupted: got {} of {}", human_size(done), human_size(total)));
    }
    if is_zip(dest) && zip::ZipArchive::new(std::fs::File::open(dest).map_err(|e| e.to_string())?).is_err() {
        return Err(format!("Download incomplete: the zip isn't whole ({} received)", human_size(done)));
    }
    // The source's own hash (mod.io md5, a link's pinned sha256) is not enforced: mod.io's stored
    // hashes and sizes are sometimes off for files its servers have served unchanged for months,
    // so a mismatch failed good downloads. The zip's own checksums (CRC32, checked while
    // unpacking) catch real damage; the size check above catches cut-off downloads.
    let sha256 = hex::encode(sha.finalize());
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

/// Content mods: the archive's top-level folders become folders in the content folder.
/// Loose files get wrapped in a folder named after the mod.
pub fn place_content(staging: &Path, mods_dir: &Path, fallback: &str) -> Result<Vec<String>, String> {
    std::fs::create_dir_all(mods_dir).map_err(|e| format!("Can't create the mods folder: {e}"))?;
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

fn is_meta(file_name: &str) -> bool {
    let lower = file_name.to_ascii_lowercase();
    PACKAGE_META.iter().any(|m| lower.starts_with(m))
}

/// Packages that wrap everything in one folder: look inside it (unless it's a folder the loader knows).
fn unwrap_single(staging: &Path, known: impl Fn(&str) -> bool) -> Result<PathBuf, String> {
    let mut base = staging.to_path_buf();
    loop {
        let items = entries(&base)?;
        let files: Vec<_> = items.iter().filter(|e| !(e.path().is_file() && is_meta(&e.file_name().to_string_lossy()))).collect();
        match files.as_slice() {
            [only] if only.path().is_dir() && !known(&only.file_name().to_string_lossy()) => base = only.path(),
            _ => return Ok(base),
        }
    }
}

/// Where a package file goes in the game folder (None: not for the game).
fn melon_dest(parts: &[String]) -> Option<Vec<String>> {
    let file_name = parts.last()?.clone();
    let is_dll = file_name.to_ascii_lowercase().ends_with(".dll");
    if parts.len() == 1 {
        // loose files at the top: DLLs are mods; manifest.json, icon.png, README… are for the website
        return is_dll.then(|| vec!["Mods".into(), file_name]);
    }
    if let Some(root) = melon_root(&parts[0]) {
        let mut v = vec![root.to_string()];
        v.extend(parts[1..].iter().cloned());
        return Some(v);
    }
    is_dll.then(|| vec!["Mods".into(), file_name])
}

/// r2modman's rules: `BepInEx/…` as is; `plugins/`, `patchers/`, `monomod/` into
/// `BepInEx/<that>/<package>/`; `config/`, `core/` into `BepInEx/<that>/`; anything else into
/// `BepInEx/plugins/<package>/`.
fn bepinex_dest(parts: &[String], package: &str) -> Option<Vec<String>> {
    let first = parts.first()?.to_ascii_lowercase();
    if parts.len() == 1 && is_meta(&parts[0]) {
        return None;
    }
    let mut v: Vec<String> = Vec::new();
    if parts.len() > 1 && first == "bepinex" {
        v.push("BepInEx".into());
        v.extend(parts[1..].iter().cloned());
    } else if parts.len() > 1 && BEPINEX_PER_PACKAGE.contains(&first.as_str()) {
        v.extend(["BepInEx".to_string(), first, package.to_string()]);
        v.extend(parts[1..].iter().cloned());
    } else if parts.len() > 1 && BEPINEX_SHARED.contains(&first.as_str()) {
        v.extend(["BepInEx".to_string(), first]);
        v.extend(parts[1..].iter().cloned());
    } else {
        v.extend(["BepInEx".to_string(), "plugins".to_string(), package.to_string()]);
        v.extend(parts.iter().cloned());
    }
    Some(v)
}

/// Code mods: files go into the game folder in the loader's layout. Returns the files placed,
/// relative to the game folder with `/` separators. `package` names the mod's own folder
/// (Thunderstore `Namespace-Name`).
pub fn place_code(staging: &Path, game_dir: &Path, loader: &str, package: &str) -> Result<Vec<String>, String> {
    let lower_pkg = package.to_ascii_lowercase();
    // BepInExPack itself: its folder is the game folder's new contents (winhttp.dll, doorstop, BepInEx/)
    let is_pack = loader == "bepinex" && lower_pkg.contains("bepinexpack");
    let base = match loader {
        "melonloader" => unwrap_single(staging, |n| melon_root(n).is_some())?,
        _ if is_pack => unwrap_single(staging, |_| false)?,
        _ => unwrap_single(staging, |n| {
            let n = n.to_ascii_lowercase();
            n == "bepinex" || BEPINEX_PER_PACKAGE.contains(&n.as_str()) || BEPINEX_SHARED.contains(&n.as_str())
        })?,
    };
    let mut files = Vec::new();
    walk(&base, &base, &mut files).map_err(|e| e.to_string())?;
    let mut placed = Vec::new();
    for rel in files {
        let parts: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
        let dest = match loader {
            "melonloader" => melon_dest(&parts),
            _ if is_pack => (!(parts.len() == 1 && is_meta(&parts[0]))).then(|| parts.clone()),
            "bepinex" => bepinex_dest(&parts, package),
            other => return Err(format!("Unknown mod loader \"{other}\"")),
        };
        let Some(dest) = dest else { continue };
        let target = dest.iter().fold(game_dir.to_path_buf(), |p, s| p.join(s));
        move_path(&base.join(&rel), &target)?;
        placed.push(dest.join("/"));
    }
    if placed.is_empty() {
        return Err("This package has nothing to install for the game".into());
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

pub fn root_dir(paths: &GamePaths, root: Root) -> PathBuf {
    match root {
        Root::Content => PathBuf::from(&paths.content_dir),
        Root::Game => PathBuf::from(&paths.game_dir),
    }
}

/// Every file inside the installed paths (relative to `root`, `/`-separated): what `is_present`
/// checks later. A path that is itself a file lists itself.
pub fn manifest(root: &Path, paths: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for rel in paths {
        let Some(p) = safe_join(root, rel) else { continue };
        if p.is_dir() {
            let mut files = Vec::new();
            if walk(&p, root, &mut files).is_ok() {
                out.extend(files.iter().map(|f| f.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/")));
            }
        } else if p.is_file() {
            out.push(rel.clone());
        }
    }
    out.sort();
    out
}

/// Every file or folder of the install is still there, and every file that was installed into
/// them. Older installs without a file list at least need their folders to have something in them.
pub fn is_present(paths: &GamePaths, inst: &Installed) -> bool {
    let root = root_dir(paths, inst.root);
    let there = |rel: &String| safe_join(&root, rel).is_some_and(|p| p.exists());
    if inst.paths.is_empty() || !inst.paths.iter().all(there) {
        return false;
    }
    if !inst.files.is_empty() {
        return inst.files.iter().all(|f| safe_join(&root, f).is_some_and(|p| p.is_file()));
    }
    inst.paths.iter().all(|rel| {
        let p = root.join(rel);
        !p.is_dir() || std::fs::read_dir(&p).is_ok_and(|mut d| d.next().is_some())
    })
}

/// Folders a loader owns: never deleted, even when a removal empties them.
fn is_loader_dir(root: &Path, dir: &Path) -> bool {
    let Ok(rel) = dir.strip_prefix(root) else { return true };
    let parts: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().to_ascii_lowercase()).collect();
    match parts.as_slice() {
        [] => true,
        [one] => melon_root(one).is_some() || one == "bepinex",
        [b, sub] => b == "bepinex" && (BEPINEX_PER_PACKAGE.contains(&sub.as_str()) || BEPINEX_SHARED.contains(&sub.as_str())),
        _ => false,
    }
}

/// Deletes an install's files; for code mods also the folders that became empty
/// (but never the loader's own folders).
pub fn remove(paths: &GamePaths, inst: &Installed) -> Result<(), String> {
    let root = root_dir(paths, inst.root);
    if root.as_os_str().is_empty() {
        return Err("Set this game's folders first (Edit game)".into());
    }
    for rel in &inst.paths {
        let Some(path) = safe_join(&root, rel) else { continue };
        remove_any(&path).map_err(|e| format!("Can't delete {}: {e}", path.display()))?;
        if inst.root == Root::Content {
            // BONELAB writes `<pallet folder>.manifest` next to each mod it loads; it goes too
            let _ = std::fs::remove_file(root.join(format!("{rel}.manifest")));
        }
        if inst.root == Root::Game {
            let mut dir = path.parent().map(Path::to_path_buf);
            while let Some(d) = dir {
                if is_loader_dir(&root, &d) || std::fs::remove_dir(&d).is_err() {
                    break;
                }
                dir = d.parent().map(Path::to_path_buf);
            }
        }
    }
    Ok(())
}

fn human_size(n: u64) -> String {
    if n >= 1 << 20 { format!("{:.0} MB", n as f64 / (1u64 << 20) as f64) } else { format!("{} KB", n.div_ceil(1024)) }
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Unmanaged {
    /// `content/<folder>` or `game/<path>`
    pub key: String,
    pub name: String,
    pub path: String,
    pub is_dir: bool,
}

/// What's in the mod folders that no mod on the list put there.
pub fn unmanaged(paths: &GamePaths, loader: &str, local: &LocalGame) -> Vec<Unmanaged> {
    let mut out = Vec::new();
    let owned = |root: Root| -> Vec<String> {
        local
            .installed
            .values()
            .filter(|i| i.root == root)
            .flat_map(|i| i.paths.iter().map(|p| p.to_ascii_lowercase()))
            .collect()
    };
    if !paths.content_dir.is_empty() {
        let owned = owned(Root::Content);
        if let Ok(items) = entries(Path::new(&paths.content_dir)) {
            for e in items {
                let name = e.file_name().to_string_lossy().into_owned();
                let key = format!("content/{name}");
                // `<folder>.manifest`: BONELAB's own bookkeeping for a mod folder it loaded, not a mod
                let game_file = name.to_ascii_lowercase().ends_with(".manifest") && e.path().is_file();
                if name.starts_with('.') || game_file || owned.contains(&name.to_ascii_lowercase()) || local.ignored.contains(&key) {
                    continue;
                }
                out.push(Unmanaged { key, name, path: e.path().to_string_lossy().into_owned(), is_dir: e.path().is_dir() });
            }
        }
    }
    if !paths.game_dir.is_empty() && loader != "none" {
        let owned = owned(Root::Game);
        let dirs: &[&str] = if loader == "melonloader" { &["Mods", "Plugins"] } else { &["BepInEx/plugins", "BepInEx/patchers"] };
        for sub in dirs {
            let Ok(items) = entries(&Path::new(&paths.game_dir).join(sub)) else { continue };
            for e in items {
                let name = e.file_name().to_string_lossy().into_owned();
                let is_dir = e.path().is_dir();
                if !is_dir && !name.to_ascii_lowercase().ends_with(".dll") {
                    continue;
                }
                let rel = format!("{sub}/{name}");
                let key = format!("game/{rel}");
                let lower = rel.to_ascii_lowercase();
                // a package folder is ours when any file we placed is inside it
                let ours = owned.iter().any(|p| *p == lower || p.starts_with(&format!("{lower}/")));
                if ours || local.ignored.contains(&key) {
                    continue;
                }
                out.push(Unmanaged { key, name: rel, path: e.path().to_string_lossy().into_owned(), is_dir });
            }
        }
    }
    out
}

pub fn delete_unmanaged(paths: &GamePaths, key: &str) -> Result<(), String> {
    let (root, rel) = if let Some(rel) = key.strip_prefix("content/") {
        (PathBuf::from(&paths.content_dir), rel)
    } else if let Some(rel) = key.strip_prefix("game/") {
        (PathBuf::from(&paths.game_dir), rel)
    } else {
        return Err("Unknown file".into());
    };
    if root.as_os_str().is_empty() {
        return Err("Unknown file".into());
    }
    let path = safe_join(&root, rel).ok_or("Unknown file")?;
    remove_any(&path).map_err(|e| format!("Can't delete {}: {e}", path.display()))
}

/// The game's mod loader is set up in its folder (always true without one).
pub fn loader_installed(paths: &GamePaths, loader: &str) -> bool {
    let game = Path::new(&paths.game_dir);
    match loader {
        "melonloader" => !paths.game_dir.is_empty() && game.join("MelonLoader").is_dir(),
        "bepinex" => !paths.game_dir.is_empty() && game.join("BepInEx").join("core").is_dir(),
        _ => true,
    }
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

    fn write(base: &Path, rel: &str) {
        let p = base.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, b"x").unwrap();
    }

    fn installed(root: Root, paths: Vec<String>) -> Installed {
        Installed { name: "x".into(), pin: String::new(), version: String::new(), root, paths, installed_at: chrono::Utc::now(), files: Vec::new() }
    }

    #[test]
    fn code_mods_follow_melonloader_layout() {
        let staging = tmp("ml-staging");
        let game = tmp("ml-game");
        write(&staging, "Mods/Foo.dll");
        write(&staging, "userlibs/Lib.dll");
        write(&staging, "Loose.dll");
        write(&staging, "manifest.json");
        let mut placed = place_code(&staging, &game, "melonloader", "Ns-Foo").unwrap();
        placed.sort();
        assert_eq!(placed, vec!["Mods/Foo.dll", "Mods/Loose.dll", "UserLibs/Lib.dll"]);
        assert!(game.join("Mods/Foo.dll").is_file());
        assert!(!game.join("manifest.json").exists());

        let paths = GamePaths { game_dir: game.to_string_lossy().into(), ..Default::default() };
        let inst = installed(Root::Game, placed);
        assert!(is_present(&paths, &inst));
        remove(&paths, &inst).unwrap();
        assert!(!game.join("Mods/Foo.dll").exists());
        assert!(game.join("Mods").is_dir(), "MelonLoader's own folders stay");
    }

    #[test]
    fn code_mods_follow_bepinex_layout() {
        let staging = tmp("bx-staging");
        let game = tmp("bx-game");
        write(&staging, "Loose.dll");
        write(&staging, "plugins/Sub/Thing.dll");
        write(&staging, "config/foo.cfg");
        write(&staging, "assets/bundle");
        write(&staging, "icon.png");
        let mut placed = place_code(&staging, &game, "bepinex", "Ns-Cool").unwrap();
        placed.sort();
        assert_eq!(
            placed,
            vec![
                "BepInEx/config/foo.cfg",
                "BepInEx/plugins/Ns-Cool/Loose.dll",
                "BepInEx/plugins/Ns-Cool/Sub/Thing.dll",
                "BepInEx/plugins/Ns-Cool/assets/bundle",
            ]
        );
        let paths = GamePaths { game_dir: game.to_string_lossy().into(), ..Default::default() };
        // a stray plugin folder is noticed, ours isn't
        write(&game, "BepInEx/plugins/Stray/Stray.dll");
        let local = LocalGame { installed: [("k".to_string(), installed(Root::Game, placed.clone()))].into(), ..Default::default() };
        let keys: Vec<String> = unmanaged(&paths, "bepinex", &local).into_iter().map(|u| u.key).collect();
        assert_eq!(keys, vec!["game/BepInEx/plugins/Stray"]);

        remove(&paths, &installed(Root::Game, placed)).unwrap();
        assert!(!game.join("BepInEx/plugins/Ns-Cool").exists(), "the package's own folder goes");
        assert!(game.join("BepInEx/plugins").is_dir() && game.join("BepInEx/config").is_dir(), "BepInEx's folders stay");
    }

    #[test]
    fn bepinex_pack_installs_into_the_game_folder() {
        let staging = tmp("pack-staging");
        let game = tmp("pack-game");
        write(&staging, "BepInExPack/winhttp.dll");
        write(&staging, "BepInExPack/BepInEx/core/BepInEx.dll");
        write(&staging, "manifest.json");
        let mut placed = place_code(&staging, &game, "bepinex", "BepInEx-BepInExPack").unwrap();
        placed.sort();
        assert_eq!(placed, vec!["BepInEx/core/BepInEx.dll", "winhttp.dll"]);
        let paths = GamePaths { game_dir: game.to_string_lossy().into(), ..Default::default() };
        assert!(loader_installed(&paths, "bepinex"));
    }

    #[test]
    fn content_folders_are_moved_and_loose_files_wrapped() {
        let staging = tmp("content-staging");
        let mods = tmp("content-mods");
        write(&staging, "Author.Pallet/pallet.json");
        assert_eq!(place_content(&staging, &mods, "x").unwrap(), vec!["Author.Pallet"]);
        assert!(mods.join("Author.Pallet/pallet.json").is_file());

        let staging = tmp("content-staging2");
        write(&staging, "thing.bin");
        assert_eq!(place_content(&staging, &mods, "My Mod").unwrap(), vec!["My Mod"]);
        assert!(mods.join("My Mod/thing.bin").is_file());
    }

    #[test]
    fn stored_paths_cannot_escape() {
        assert!(safe_join(Path::new("C:/x"), "../evil").is_none());
        assert!(safe_join(Path::new("C:/x"), "").is_none());
        assert!(safe_join(Path::new("C:/x"), "Mods/a.dll").is_some());
    }

    #[test]
    fn content_mod_with_a_missing_file_is_not_present() {
        let mods = tmp("manifest");
        write(&mods, "Author.Pallet/pallet.json");
        write(&mods, "Author.Pallet/sub/level.bundle");
        let paths = GamePaths { content_dir: mods.to_string_lossy().into(), ..Default::default() };
        let mut inst = installed(Root::Content, vec!["Author.Pallet".into()]);
        inst.files = manifest(&mods, &inst.paths);
        assert_eq!(inst.files, vec!["Author.Pallet/pallet.json", "Author.Pallet/sub/level.bundle"]);
        assert!(is_present(&paths, &inst));
        std::fs::remove_file(mods.join("Author.Pallet/sub/level.bundle")).unwrap();
        assert!(!is_present(&paths, &inst), "the folder is there, but a file of the mod isn't");
    }

    #[test]
    fn older_install_with_an_empty_folder_is_not_present() {
        let mods = tmp("empty-folder");
        std::fs::create_dir_all(mods.join("Old.Mod")).unwrap();
        let paths = GamePaths { content_dir: mods.to_string_lossy().into(), ..Default::default() };
        let inst = installed(Root::Content, vec!["Old.Mod".into()]);
        assert!(!is_present(&paths, &inst));
        write(&mods, "Old.Mod/a.bundle");
        assert!(is_present(&paths, &inst));
    }

    #[test]
    fn game_manifests_are_not_unmanaged_and_go_with_their_mod() {
        let mods = tmp("manifests");
        write(&mods, "BaBaCorp.711/pallet.json");
        write(&mods, "BaBaCorp.711.manifest");
        write(&mods, "Someone.Else/pallet.json");
        let paths = GamePaths { content_dir: mods.to_string_lossy().into(), ..Default::default() };
        let mut local = LocalGame::default();
        let inst = installed(Root::Content, vec!["BaBaCorp.711".into()]);
        local.installed.insert("modio:1".into(), inst.clone());
        let names: Vec<String> = unmanaged(&paths, "none", &local).into_iter().map(|u| u.name).collect();
        assert_eq!(names, vec!["Someone.Else"]);
        remove(&paths, &inst).unwrap();
        assert!(!mods.join("BaBaCorp.711.manifest").exists());
    }
}
