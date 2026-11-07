import { reactive } from "vue";
import { listen } from "@tauri-apps/api/event";
import { api, type NewMod, type Overview, type Progress, type Settings } from "./api";
import { errorText, toast } from "./toast";

/** Everything the window shows, kept fresh by the backend's events. */
export const store = reactive({
  settings: null as Settings | null,
  overview: null as Overview | null,
  error: "",
  loading: false,
  live: false,
  syncing: false,
  progress: null as Progress | null,
  /** newer files found by "Check for updates", by mod id */
  updates: {} as Record<string, NewMod>,
  checking: false,
  checkedAt: 0,
});

export const ONLINE_MS = 150_000;

export async function loadSettings() {
  store.settings = await api.settings();
}

export function setupDone(s: Settings | null = store.settings): boolean {
  return !!s && !!s.name.trim() && !!s.databaseUrl.trim() && !!s.modsDir.trim();
}

let refreshing: Promise<void> | null = null;
export function refresh(): Promise<void> {
  if (!setupDone()) return Promise.resolve();
  // one at a time; a refresh asked for meanwhile reuses the running one
  refreshing ??= (async () => {
    store.loading = true;
    try {
      store.overview = await api.overview();
      store.live = store.overview.live;
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
  if (store.syncing) return;
  store.syncing = true;
  store.progress = null;
  try {
    const r = await api.sync();
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
  if (store.checking) return;
  store.checking = true;
  try {
    store.updates = await api.checkUpdates();
    store.checkedAt = Date.now();
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
    let change: { actor?: string; action?: string; name?: string } = {};
    try {
      change = JSON.parse(e.payload);
    } catch {
      /* an older or foreign payload: just refresh */
    }
    await refresh();
    const mine = change.actor === store.overview?.me;
    if (mine || !change.action || !VERBS[change.action]) return;
    toast.info(`${change.actor} ${VERBS[change.action]} ${change.name}`);
    if (store.settings?.autoSync && store.overview?.plan.length) void runSync();
  });
  // presence: tell the other app we're here, see if they are
  setInterval(() => void refresh(), 60_000);
}
