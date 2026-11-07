import { invoke } from "@tauri-apps/api/core";

export interface GamePaths {
  contentDir: string;
  gameDir: string;
}

export interface Settings {
  name: string;
  databaseUrl: string;
  modioApiKey: string;
  modioToken: string;
  autoSync: boolean;
  currentGame: number | null;
  games: Record<string, GamePaths>;
}

export type Loader = "melonloader" | "bepinex" | "none";

export interface NewGame {
  name: string;
  modioId: number;
  modioSlug: string;
  thunderstore: string;
  loader: Loader;
  steamDir: string;
  contentHint: string;
}

export interface Game extends NewGame {
  id: number;
  createdBy: string;
}

export interface GameView extends Game {
  mods: number;
  paths: GamePaths;
}

export type Source = "modio" | "thunderstore" | "link";
export type Target = "content" | "code";

export interface NewMod {
  source: Source;
  sourceId: string;
  target: Target;
  name: string;
  version: string;
  fileRef: string;
  fileHash: string;
  fileSize: number;
  pageUrl: string;
  thumbUrl: string;
  summary: string;
  author: string;
  extra: Record<string, unknown>;
}

export interface SharedMod extends NewMod {
  id: number;
  gameId: number;
  addedBy: string;
  addedAt: string;
  updatedBy: string;
  updatedAt: string;
}

export type LocalState = "installed" | "missing" | "outdated" | "broken";

export interface ModView extends SharedMod {
  state: LocalState;
  localVersion: string | null;
}

export type Action = "install" | "update" | "repair" | "remove";

export interface PlanItem {
  key: string;
  id: number | null;
  name: string;
  action: Action;
  from: string | null;
  to: string | null;
  size: number;
  target: Target;
}

export interface Member {
  name: string;
  lastSeen: string;
  /** null: hasn't opened this game */
  pending: number | null;
  syncedAt: string | null;
}

export interface Overview {
  me: string;
  game: Game;
  paths: GamePaths;
  mods: ModView[];
  plan: PlanItem[];
  members: Member[];
  unmanaged: number;
  loaderOk: boolean;
  needsLoader: boolean;
  live: boolean;
}

export interface Resolved {
  main: NewMod;
  deps: NewMod[];
}

export interface SearchHit {
  id: string;
  input: string;
  name: string;
  summary: string;
  thumb: string;
  author: string;
  downloads: number;
}

export interface ModEvent {
  id: number;
  at: string;
  actor: string;
  action: string;
  modName: string;
  detail: string;
}

export interface Unmanaged {
  key: string;
  name: string;
  path: string;
  isDir: boolean;
}

export interface Progress {
  game: number;
  index: number;
  count: number;
  name: string;
  action: Action;
  phase: "downloading" | "installing" | "removing" | "done";
  done: number;
  total: number;
}

export interface SyncResult {
  done: number;
  failed: [string, string][];
}

export interface Invite {
  databaseUrl: string;
  game: number | null;
}

export const LOADER_LABEL: Record<Loader, string> = { melonloader: "MelonLoader", bepinex: "BepInEx", none: "No mod loader" };

export const api = {
  settings: () => invoke<Settings>("get_settings"),
  saveSettings: (settings: Settings) => invoke<void>("save_settings", { settings }),
  detectFolders: (steamDir: string, contentHint: string) => invoke<GamePaths>("detect_folders", { steamDir, contentHint }),
  testConnection: (url: string) => invoke<void>("test_connection", { url }),
  inviteCode: (game: number | null) => invoke<string>("invite_code", { game }),
  readInvite: (code: string) => invoke<Invite>("read_invite", { code }),

  presets: () => invoke<NewGame[]>("presets"),
  games: () => invoke<GameView[]>("list_games"),
  createGame: (game: NewGame, paths: GamePaths) => invoke<number>("create_game", { game, paths }),
  updateGame: (id: number, game: NewGame, paths: GamePaths) => invoke<void>("update_game", { id, game, paths }),
  setGamePaths: (id: number, paths: GamePaths) => invoke<void>("set_game_paths", { id, paths }),
  deleteGame: (id: number) => invoke<void>("delete_game", { id }),
  setCurrentGame: (id: number | null) => invoke<void>("set_current_game", { id }),

  overview: (game: number) => invoke<Overview>("overview", { game }),
  resolve: (game: number, input: string, name?: string, target?: Target) => invoke<Resolved>("resolve_mod", { game, input, name, target }),
  search: (game: number, query: string) => invoke<SearchHit[]>("search_modio", { game, query }),
  add: (game: number, mods: NewMod[]) => invoke<number>("add_mods", { game, mods }),
  remove: (id: number) => invoke<void>("remove_mod", { id }),
  checkUpdates: (game: number) => invoke<Record<string, NewMod>>("check_updates", { game }),
  update: (id: number) => invoke<string | null>("update_mod", { id }),
  events: (game: number) => invoke<ModEvent[]>("events", { game }),
  sync: (game: number) => invoke<SyncResult>("sync_now", { game }),
  unmanaged: (game: number) => invoke<Unmanaged[]>("unmanaged", { game }),
  deleteUnmanaged: (game: number, keys: string[]) => invoke<void>("delete_unmanaged", { game, keys }),
  ignoreUnmanaged: (game: number, keys: string[], ignore = true) => invoke<void>("ignore_unmanaged", { game, keys, ignore }),
  importLegacy: (game: number) => invoke<[number, string[]]>("import_legacy", { game }),
  openFolder: (game: number, which: "content" | "game") => invoke<void>("open_folder", { game, which }),
  openUrl: (url: string) => invoke<void>("open_url", { url }),
  version: () => invoke<string>("app_version"),
};

/** Example address for the database fields (bound, not inline: vue-tsc mis-parses the // in a static attribute). */
export const DB_PLACEHOLDER = "postgres://user:password@host:5432/mods";
