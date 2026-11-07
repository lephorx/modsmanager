import { invoke } from "@tauri-apps/api/core";

export interface Settings {
  name: string;
  databaseUrl: string;
  modioApiKey: string;
  modioToken: string;
  gameId: number;
  modsDir: string;
  gameDir: string;
  autoSync: boolean;
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
  pending: number;
  syncedAt: string | null;
}

export interface Overview {
  me: string;
  mods: ModView[];
  plan: PlanItem[];
  members: Member[];
  unmanaged: number;
  melon: boolean;
  needsMelon: boolean;
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

export const api = {
  settings: () => invoke<Settings>("get_settings"),
  saveSettings: (settings: Settings) => invoke<void>("save_settings", { settings }),
  detectFolders: () => invoke<{ modsDir: string | null; gameDir: string | null }>("detect_folders"),
  testConnection: (url: string) => invoke<void>("test_connection", { url }),
  overview: () => invoke<Overview>("overview"),
  resolve: (input: string, name?: string, target?: Target) => invoke<Resolved>("resolve_mod", { input, name, target }),
  search: (query: string) => invoke<SearchHit[]>("search_modio", { query }),
  add: (mods: NewMod[]) => invoke<number>("add_mods", { mods }),
  remove: (id: number) => invoke<void>("remove_mod", { id }),
  checkUpdates: () => invoke<Record<string, NewMod>>("check_updates"),
  update: (id: number) => invoke<string | null>("update_mod", { id }),
  events: () => invoke<ModEvent[]>("events"),
  sync: () => invoke<SyncResult>("sync_now"),
  unmanaged: () => invoke<Unmanaged[]>("unmanaged"),
  deleteUnmanaged: (keys: string[]) => invoke<void>("delete_unmanaged", { keys }),
  ignoreUnmanaged: (keys: string[], ignore = true) => invoke<void>("ignore_unmanaged", { keys, ignore }),
  importLegacy: () => invoke<[number, string[]]>("import_legacy"),
  openFolder: (which: "mods" | "game") => invoke<void>("open_folder", { which }),
  openUrl: (url: string) => invoke<void>("open_url", { url }),
  version: () => invoke<string>("app_version"),
};
