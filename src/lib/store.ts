import { reactive } from "vue";
import { listen } from "@tauri-apps/api/event";
import { api, type GameView, type NewMod, type Overview, type Progress, type Settings } from "./api";
import { errorText, toast } from "./toast";

/** Everything the window shows, kept fresh by the backend's events. */
export const store = reactive({
  settings: null as Settings | null,
  /** every game in the shared database */
  games: null as GameView[] | null,
  /** the shown game's list, people and plan */
  overview: null as Overview | null,
  error: "",
  loading: false,
  live: false,
  syncing: false,
  progress: null as Progress | null,
  /** newer files found by "Check for updates", by mod id (for the shown game) */
  updates: {} as Record<string, NewMod>,
  checking: false,
});

export const ONLINE_MS = 150_000;

export async function loadSettings() {
  store.settings = await api.settings();
}

export function setupDone(s: Settings | null = store.settings): boolean {
  return !!s && !!s.name.trim() && !!s.databaseUrl.trim();
}

export function currentGame(): number | null {
  return store.settings?.currentGame ?? null;
}

/** This computer has folders for the game. */
export function hasPaths(id: number | null = currentGame()): boolean {
  const p = id == null ? undefined : store.settings?.games[String(id)];
  return !!p && (!!p.contentDir.trim() || !!p.gameDir.trim());
}

export async function selectGame(id: number | null) {
  if (!store.settings || store.settings.currentGame === id) return;
  await api.setCurrentGame(id);
  store.settings.currentGame = id;
  store.overview = null;
  store.updates = {};
  await refresh();
}

let refreshing: Promise<void> | null = null;
export function refresh(): Promise<void> {
  if (!setupDone()) return Promise.resolve();
  // one at a time; a refresh asked for meanwhile reuses the running one
  refreshing ??= (async () => {
    store.loading = true;
    try {
      store.games = await api.games();
      // after the first database call: that's when the backend moves a v0.1.0 setup over
      store.settings = await api.settings();
      let id = currentGame();
      // the game was removed, or none is picked yet: take the first one
      if (id == null || !store.games.some((g) => g.id === id)) {
        id = store.games[0]?.id ?? null;
        if (store.settings && store.settings.currentGame !== id) {
          await api.setCurrentGame(id);
          store.settings.currentGame = id;
          store.overview = null;
          store.updates = {};
        }
      }
      store.overview = id == null ? null : await api.overview(id);
      if (store.overview) store.live = store.overview.live;
      store.error = "";
    } catch (e) {
      store.error = errorText(e);
    } finally {
      store.loading = false;
      refreshing = null;
    }
  })();
  return refreshing;
}

export async function runSync() {
  const game = currentGame();
  if (store.syncing || game == null) return;
  store.syncing = true;
  store.progress = null;
  try {
    const r = await api.sync(game);
    if (r.failed.length) {
      toast.error(`${r.failed.length} couldn't sync: ${r.failed.map(([n, e]) => `${n} (${e})`).join("; ")}`);
    } else if (r.done) {
      toast.success(r.done === 1 ? "1 change synced" : `${r.done} changes synced`);
    }
  } catch (e) {
    toast.error(errorText(e));
  } finally {
    store.syncing = false;
    store.progress = null;
    await refresh();
  }
}

export async function checkUpdates(quiet = false) {
  const game = currentGame();
  if (store.checking || game == null) return;
  store.checking = true;
  try {
    store.updates = await api.checkUpdates(game);
    const n = Object.keys(store.updates).length;
    if (!quiet) n ? toast.info(n === 1 ? "1 mod has a newer version" : `${n} mods have newer versions`) : toast.success("Everything is on its newest version");
  } catch (e) {
    if (!quiet) toast.error(errorText(e));
  } finally {
    store.checking = false;
  }
}

const VERBS: Record<string, string> = { add: "added", update: "updated", remove: "removed", import: "imported" };

let started = false;
export async function startEvents() {
  if (started) return;
  started = true;
  await listen<Progress>("sync-progress", (e) => {
    store.progress = e.payload.phase === "done" ? null : e.payload;
  });
  await listen<boolean>("live", (e) => (store.live = e.payload));
  await listen<string>("remote-changed", async (e) => {
    let change: { game?: number; actor?: string; action?: string; name?: string } = {};
    try {
      change = JSON.parse(e.payload);
    } catch {
      /* an older app's payload: just refresh */
    }
    await refresh();
    const mine = change.actor === store.settings?.name.trim();
    if (mine || !change.action) return;
    if (change.action === "game") return toast.info(`${change.actor} set up ${change.name}`);
    if (change.action === "game-removed") return toast.info(`${change.actor} removed ${change.name}`);
    if (!VERBS[change.action]) return;
    const here = change.game == null || change.game === currentGame();
    const gameName = store.games?.find((g) => g.id === change.game)?.name;
    toast.info(`${change.actor} ${VERBS[change.action]} ${change.name}${!here && gameName ? ` in ${gameName}` : ""}`);
    if (here && store.settings?.autoSync && store.overview?.plan.length) void runSync();
  });
  // presence: tell the others we're here, see who else is
  setInterval(() => void refresh(), 60_000);
}
