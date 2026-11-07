<img src="assets/icon.svg" width="64" height="64" alt="">

# Mods Manager

Keeps everyone's mods **identical**: same mods, same exact files, for any game and any number of people.

- One **shared database** everyone connects to, holding any number of **games**, each with its own mod list.
- Every mod is **pinned to one exact file**: a mod.io modfile, a Thunderstore version, or the SHA-256 of a downloaded file. "In sync" means byte-for-byte the same, not "everyone on latest".
- When someone adds, updates or removes a mod, everyone's app hears it **live** (`LISTEN/NOTIFY`), shows a toast and a **Sync now** card listing exactly what to install, update or remove. Turn on *Settings → Syncing → Sync on its own* to skip the click.
- New versions don't sneak in: **Check for updates** shows what's newer, and **Update** re-pins it for everyone.
- **People**: everyone who joined, online or not, and whether they're in sync in the game you're looking at.
- **Invite codes**: one paste sets up someone's app (database and game). The database address, API key, token and invite codes are hidden fields.
- **Activity** shows who changed what, and when. Files in a game's folders that aren't on its list get flagged so you can delete or keep them.
- **Install queue**: Sync, new mods, auto-sync and Nexus clicks all go into one queue; 3 downloads at a time, each with its own progress. The Downloads panel shows what's running, waiting for a Nexus click, queued, done or failed (retry, cancel, clear).
- Updates itself from GitHub releases.

Built with Tauri 2 (Rust) and Vue 3, in Tessera Launcher's design (Modern and Classic looks). Windows only for now.

## Games

Presets for **BONELAB**, **Blade & Sorcery** and **Lethal Company**; any other game works too. A game says where its mods come from and how code mods load:

| Setting | Example |
| --- | --- |
| mod.io game | `bladeandsorcery` (the id is looked up for you) |
| Nexus Mods game | `bladeandsorcery`, as in nexusmods.com/bladeandsorcery |
| Thunderstore community | `lethal-company` |
| Code mods | MelonLoader, BepInEx, or none |

| Source | Paste | Goes to |
| --- | --- | --- |
| mod.io | the mod's page, a mod id, or search in the app | the game's mods folder |
| Thunderstore | the package page | the game folder in the loader's layout: MelonLoader (`Mods/`, `Plugins/`, `UserLibs/`, `UserData/`) or BepInEx (`BepInEx/plugins/<package>/…`, like r2modman; BepInExPack itself installs into the game folder). Dependencies come along. |
| Nexus Mods | the mod's page (newest main file), or a file link with `?tab=files&file_id=…` | the mods folder or the game folder, you choose |
| Direct link | any `https://…` file | the mods folder or the game folder, you choose |

mod.io downloads are checked against mod.io's MD5, links against the pinned SHA-256. Each computer sets its own folders per game (found automatically for Steam installs).

### Nexus Mods without Premium

Nexus only gives API download links to Premium accounts, or to a free account right after it clicked **Mod Manager Download** on the website. So with a free account (add its personal API key in *Settings → Nexus Mods*):

1. **Sync now** installs everything from mod.io, Thunderstore and links, and lists the Nexus mods that need a click.
2. **Get from Nexus** opens the exact pinned file's page; click **Slow download**.
3. Nexus hands the click to Mods Manager (`nxm://` link), which downloads exactly that file, installs it, and opens the next one.

With Premium, Nexus mods sync on their own. Mods Manager opens `nxm://` links while a Nexus key is set; turn that off in Settings if Vortex or Mod Organizer should get them. If the browser doesn't open the app, copy the "Mod Manager Download" link and paste it in *Settings → Nexus Mods*.

## Setup

1. Install from [Releases](../../releases).
2. Enter your name, then either paste an **invite code** from someone who already uses it, or a **Postgres address** (`postgres://user:password@host:5432/db`) everyone can reach. The app creates its tables (`mm_*`); TLS is used when the server offers it.
3. Add your **mod.io API key** (mod.io → Account → API access) if you play games on mod.io.
4. Add a game, or pick the one you were invited to and set its folders.

Upgrading from 0.1.0 needs nothing: the existing list becomes a BONELAB game, with your folders and installed mods, so nothing downloads again.

Coming from the old [Mod.io-Mods-Manager](https://github.com/lephorx/Mod.io-Mods-Manager)? Point this at the same database and use *Settings → Maintenance → Import*.

## Development

Needs Node 22+, pnpm and Rust (Windows: the MSVC build tools).

```sh
pnpm install
pnpm dev          # "Mods Manager Dev": its own settings, separate from the installed app
pnpm typecheck
pnpm test         # Rust unit tests
```

End-to-end tests (the 0.1.0 migration; three people, two games, real Thunderstore downloads) against a throwaway database:

```sh
MM_TEST_DB=postgres://user:pass@localhost:5432/scratch cargo test --manifest-path src-tauri/Cargo.toml -- --ignored --test-threads=1
```

They drop the `mm_*` tables, so never point them at the real database.

## Releasing

Bump the version in `package.json`, `src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json`, then push a tag:

```sh
git tag -a v0.3.0 -m "What's new" && git push origin v0.3.0
```

GitHub Actions builds the signed Windows installer and publishes the release; installed apps update themselves.
