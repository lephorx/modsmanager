//! What lives on this computer only: your settings, each game's folders here, and what this
//! copy of the app installed.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    /// Your name, shown to everyone next to what you add, update or remove.
    pub name: String,
    /// The shared Postgres database everyone connects to.
    pub database_url: String,
    pub modio_api_key: String,
    /// Optional mod.io OAuth token: some downloads need a signed-in user.
    pub modio_token: String,
    /// Nexus Mods personal API key (nexusmods.com → Site preferences → API Keys).
    pub nexus_api_key: String,
    /// Set only after the Nexus browser confirms a signed-in session.
    pub nexus_logged_in: bool,
    /// Leave `nxm://` links ("Mod Manager Download") to another mod manager such as Vortex.
    pub nexus_ignore_links: bool,
    /// Run a sync on its own when someone changes the list.
    pub auto_sync: bool,
    /// The game the window shows.
    pub current_game: Option<i64>,
    /// Where each game lives on this computer, by game id.
    pub games: BTreeMap<i64, GamePaths>,
    /// v0.1.0 had one game (BONELAB) and kept its folders here; moved into `games` once.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub mods_dir: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub game_dir: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct GamePaths {
    /// Where content mods go (folders), e.g. BONELAB's LocalLow Mods folder.
    pub content_dir: String,
    /// The game's install folder (MelonLoader / BepInEx live here).
    pub game_dir: String,
}

impl Settings {
    pub fn paths(&self, game: i64) -> GamePaths {
        self.games.get(&game).cloned().unwrap_or_default()
    }
}

/// Where an installed mod's files live.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Root {
    /// Paths are relative to the content folder (top-level folders).
    #[serde(alias = "mods")]
    Content,
    /// Paths are relative to the game folder (single files, e.g. `Mods/Foo.dll`).
    Game,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Installed {
    pub name: String,
    /// The exact pin that was installed, see `NewMod::pin`.
    pub pin: String,
    pub version: String,
    pub root: Root,
    pub paths: Vec<String>,
    pub installed_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct LocalGame {
    /// Keyed by `source:source_id`.
    pub installed: BTreeMap<String, Installed>,
    /// Unmanaged files you chose to keep (`content/<name>` or `game/<path>`).
    pub ignored: BTreeSet<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Local {
    pub games: BTreeMap<i64, LocalGame>,
    /// v0.1.0 (BONELAB only): moved into `games` once.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub installed: BTreeMap<String, Installed>,
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    pub ignored: BTreeSet<String>,
}

impl Local {
    pub fn game(&self, id: i64) -> LocalGame {
        self.games.get(&id).cloned().unwrap_or_default()
    }
    pub fn game_mut(&mut self, id: i64) -> &mut LocalGame {
        self.games.entry(id).or_default()
    }
}

pub fn load<T: for<'de> Deserialize<'de> + Default>(path: &Path) -> T {
    std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// Write to a temp file first so a crash never leaves half a file behind.
pub fn save<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

/// A game's usual folders on this computer, from its Steam folder name and content folder hint.
/// The hint may start with `%GAME%` (the install folder) or `%LOCALLOW%` (…\AppData\LocalLow).
pub fn detect(steam_dir: &str, content_hint: &str) -> GamePaths {
    let game_dir = if steam_dir.trim().is_empty() {
        None
    } else {
        steam_libraries()
            .into_iter()
            .map(|lib| lib.join("steamapps").join("common").join(steam_dir.trim()))
            .find(|p| p.is_dir())
    };
    let content_dir = expand(content_hint, game_dir.as_deref());
    GamePaths {
        content_dir: content_dir.map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
        game_dir: game_dir.map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
    }
}

fn expand(hint: &str, game_dir: Option<&Path>) -> Option<PathBuf> {
    let hint = hint.trim();
    if hint.is_empty() {
        return None;
    }
    let (base, rest) = if let Some(rest) = hint.strip_prefix("%GAME%") {
        (game_dir?.to_path_buf(), rest)
    } else if let Some(rest) = hint.strip_prefix("%LOCALLOW%") {
        (dirs::home_dir()?.join("AppData").join("LocalLow"), rest)
    } else if let Some(rest) = hint.strip_prefix("%DOCUMENTS%") {
        (dirs::document_dir()?, rest)
    } else {
        return Some(PathBuf::from(hint));
    };
    let path = rest.split(['/', '\\']).filter(|s| !s.is_empty()).fold(base, |p, s| p.join(s));
    // only suggest it when the game made its parent already
    path.parent().is_some_and(|p| p.is_dir()).then_some(path)
}

fn steam_libraries() -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    #[cfg(windows)]
    {
        use winreg::enums::HKEY_CURRENT_USER;
        if let Ok(key) = winreg::RegKey::predef(HKEY_CURRENT_USER).open_subkey("Software\\Valve\\Steam") {
            if let Ok(p) = key.get_value::<String, _>("SteamPath") {
                roots.push(PathBuf::from(p));
            }
        }
        roots.push(PathBuf::from("C:\\Program Files (x86)\\Steam"));
    }
    #[cfg(not(windows))]
    if let Some(h) = dirs::home_dir() {
        roots.push(h.join(".steam/steam"));
        roots.push(h.join(".local/share/Steam"));
    }
    let mut libs = Vec::new();
    for root in roots {
        let vdf = root.join("steamapps").join("libraryfolders.vdf");
        if let Ok(text) = std::fs::read_to_string(&vdf) {
            for line in text.lines() {
                let parts: Vec<&str> = line.split('"').collect();
                // \t"path"\t\t"D:\\SteamLibrary"
                if parts.len() >= 4 && parts[1] == "path" {
                    libs.push(PathBuf::from(parts[3].replace("\\\\", "\\")));
                }
            }
        }
        libs.push(root);
    }
    libs.dedup();
    libs
}
