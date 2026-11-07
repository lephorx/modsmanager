<img src="assets/icon.svg" width="64" height="64" alt="">

# Mods Manager

Keeps your and your friend's BONELAB mods **identical**: same mods, same exact files.

- One **shared list** in a Postgres database you both connect to.
- Every mod on the list is **pinned to one exact file**: a mod.io modfile, a Thunderstore version, or the SHA-256 of a downloaded file. "In sync" means byte-for-byte the same, not "both on latest".
- When one of you adds, updates or removes a mod, the other app hears it **live** (`LISTEN/NOTIFY`), shows a toast and a **Sync now** card listing exactly what to install, update or remove. Turn on *Settings → Syncing → Sync on its own* to skip the click.
- New versions don't sneak in: **Check for updates** shows what's newer, and **Update** re-pins it for both of you.
- The top bar shows your friend: online or not, and whether they're in sync.
- **Activity** shows who changed what, and when.
- Files in your mod folders that aren't on the list get flagged so you can delete them or keep them.

Built with Tauri 2 (Rust) and Vue 3, in Tessera Launcher's design (Modern and Classic looks).

## What it installs

| Source | Paste | Goes to |
| --- | --- | --- |
| mod.io | `https://mod.io/g/bonelab/m/<mod>`, a mod id, or search in the app | BONELAB's Mods folder (`…\AppData\LocalLow\Stress Level Zero\BONELAB\Mods`) |
| Thunderstore | `https://thunderstore.io/c/bonelab/p/<ns>/<name>/` | The game folder in MelonLoader's layout (`Mods/`, `Plugins/`, `UserLibs/`, `UserData/`). Dependencies come along; MelonLoader itself is left to its installer. |
| Direct link | Any `https://…` file | Either of the two, you choose |

mod.io downloads are checked against mod.io's MD5, links against the pinned SHA-256.

## Setup (both of you)

1. Install from [Releases](../../releases) (or build it, below).
2. On first start enter:
   - **Your name**: your friend sees it next to your changes.
   - **Shared database**: the same `postgres://user:password@host:5432/db` on both computers. The app creates its tables (`mm_mods`, `mm_events`, `mm_members`); TLS is used when the server offers it.
   - **mod.io API key**: mod.io → Account → API access. If downloads say *unauthorized*, also add an OAuth token in Settings → mod.io.
   - BONELAB's **Mods folder** and **game folder** (found automatically for Steam installs).

Coming from the old [Mod.io-Mods-Manager](https://github.com/lephorx/Mod.io-Mods-Manager)? Point this at the same database and use *Settings → Maintenance → Import from the old manager*: it copies the mod.io and Thunderstore mods onto the new list, pinned to today's files.

## Development

Needs Node 22+, pnpm and Rust (Windows: the MSVC build tools).

```sh
pnpm install
pnpm dev          # the app with hot reload
pnpm typecheck
pnpm test         # Rust unit tests
```

End-to-end test (two simulated computers, real Thunderstore downloads) against a throwaway database:

```sh
MM_TEST_DB=postgres://user:pass@localhost:5432/scratch cargo test --manifest-path src-tauri/Cargo.toml -- --ignored two_friends
```

It truncates the `mm_*` tables, so never point it at the real list.

## Releasing

Bump the version in `package.json`, `src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json`, then push a tag:

```sh
git tag v0.1.0 && git push origin v0.1.0
```

GitHub Actions builds the Windows installer and publishes the release.
