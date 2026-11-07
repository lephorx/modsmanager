//! What lives on this computer only: your settings and what this copy of the app installed.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// BONELAB on mod.io.
pub const DEFAULT_GAME_ID: u64 = 3809;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    /// Your name, shown to your friend next to everything you add or update.
    pub name: String,
    /// The shared Postgres database both of you connect to.
    pub database_url: String,
    pub modio_api_key: String,
    /// Optional mod.io OAuth token: some downloads need a signed-in user.
    pub modio_token: String,
    pub game_id: u64,
    /// BONELAB's content mods (pallets): ...\AppData\LocalLow\Stress Level Zero\BONELAB\Mods
    pub mods_dir: String,
    /// The BONELAB install folder (MelonLoader code mods go into its Mods, Plugins, UserLibs, UserData).
    pub game_dir: String,
    /// Run a sync on its own when your friend changes the list.
    pub auto_sync: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            name: String::new(),
            database_url: String::new(),
            modio_api_key: String::new(),
            modio_token: String::new(),
            game_id: DEFAULT_GAME_ID,
            mods_dir: String::new(),
            game_dir: String::new(),
            auto_sync: false,
        }
    }
}

/// Where an installed mod's files live.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Root {
    /// Paths are relative to `mods_dir` (top-level folders).
    Mods,
    /// Paths are relative to `game_dir` (single files, e.g. `Mods/Foo.dll`).
    Game,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Installed {
    pub name: String,
    /// The exact pin that was installed, see `SharedMod::pin`.
    pub pin: String,
    pub version: String,
    pub root: Root,
    pub paths: Vec<String>,
    pub installed_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Local {
    /// Keyed by `source:source_id`.
    pub installed: BTreeMap<String, Installed>,
    /// Unmanaged files you chose to keep (relative, prefixed `mods/` or `game/`).
    pub ignored: BTreeSet<String>,
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

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Detected {
    pub mods_dir: Option<String>,
    pub game_dir: Option<String>,
}

/// BONELAB's usual folders on this computer.
pub fn detect() -> Detected {
    let mods_dir = dirs::home_dir()
        .map(|h| h.join("AppData").join("LocalLow").join("Stress Level Zero").join("BONELAB").join("Mods"))
        .filter(|p| p.parent().is_some_and(|p| p.is_dir()))
        .map(|p| p.to_string_lossy().into_owned());
    let game_dir = steam_libraries()
        .into_iter()
        .map(|lib| lib.join("steamapps").join("common").join("BONELAB"))
        .find(|p| p.is_dir())
        .map(|p| p.to_string_lossy().into_owned());
    Detected { mods_dir, game_dir }
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
