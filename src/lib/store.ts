import { reactive } from "vue";
import { listen } from "@tauri-apps/api/event";
import { api, isActive, type GameView, type Job, type NewMod, type Overview, type Settings } from "./api";
import { errorText, toast } from "./toast";
import { openSettings } from "./settingsDialog";

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
  /** the install queue (every game) */
  queue: [] as Job[],
  /** something is installing or removing files (any game) */
  syncing: false,
  /** newer files found by "Check for updates", by mod id (for the shown game) */
  updates: {} as Record<string, NewMod>,
  checking: false,
  /** automating free Nexus downloads one page at a time */
  nexusAssist: false,
  /** the queue panel in the top bar */
  queueOpen: false,
});

/** The queue's jobs for the game on screen. */
export function gameJobs(game: number | null = currentGame()) {
  return store.queue.filter((j) => j.game === game);
}

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

/** Queues everything this computer has to do for the game on screen. */
export async function runSync(quiet = false) {
  const game = currentGame();
  if (game == null) return;
  try {
    const added = await api.queueSync(game);
    if (!quiet && !added && !gameJobs(game).some((j) => isActive(j) || j.status === "waiting")) toast.success("Nothing to do: this computer matches the list");
  } catch (e) {
    toast.error(errorText(e));
  }
}

/** A batch of jobs ran out: say how it went, then refresh the list. */
let batch = { done: 0, failed: 0, cancelled: 0 };
let wasActive = false;
let refreshTimer: ReturnType<typeof setTimeout> | undefined;
function onQueue(jobs: Job[]) {
  const before = new Map(store.queue.map((j) => [j.id, j.status]));
  store.queue = jobs;
  for (const j of jobs) {
    const was = before.get(j.id);
    if (was === j.status) continue;
    if (j.status === "done") batch.done++;
    if (j.status === "cancelled") batch.cancelled++;
    if (j.status === "failed") {
      batch.failed++;
      toast.error(`${j.name}: ${j.error ?? "failed"}`);
    }
  }
  const active = jobs.some(isActive);
  store.syncing = active;
  // the list's install states follow the files: refresh after changes, not on every progress tick
  if ([...before].some(([id, st]) => st !== jobs.find((j) => j.id === id)?.status)) {
    clearTimeout(refreshTimer);
    refreshTimer = setTimeout(() => void refresh(), 400);
  }
  if (wasActive && !active) {
    const parts = [batch.done && `${batch.done} installed`, batch.failed && `${batch.failed} failed`, batch.cancelled && `${batch.cancelled} cancelled`].filter(Boolean);
    if (batch.done && !batch.failed && !batch.cancelled) toast.success(batch.done === 1 ? "1 change installed" : `${batch.done} changes installed`);
    else if (parts.length) toast.info(`${parts.join(", ")}${batch.failed ? " (see Downloads)" : ""}`);
    batch = { done: 0, failed: 0, cancelled: 0 };
  }
  wasActive = active;
  if (store.nexusAssist) followNexus();
  else if (gameJobs().some((j) => j.status === "waiting" && before.get(j.id) !== "waiting")) followNexus();
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

/** Nexus files still needing a browser-generated download link. */
export function nexusPending() {
  const waiting = gameJobs().filter((j) => j.status === "waiting");
  const queuedKeys = new Set(gameJobs().map((j) => j.key));
  const notQueued = (store.overview?.plan ?? []).filter((p) => p.manualUrl && !queuedKeys.has(p.key));
  return [...waiting.map((j) => ({ key: j.key, name: j.name, manualUrl: j.manualUrl })), ...notQueued.map((p) => ({ key: p.key, name: p.name, manualUrl: p.manualUrl }))];
}

/** The Nexus job currently being prepared in the browser. */
let assisting: string | null = null;

/** Not logged in to the Nexus website: send the user to Settings to log in, and start nothing. */
export function needsNexusLogin(): boolean {
  if (store.settings?.nexusLoggedIn) return false;
  stopNexus();
  openSettings("nexus");
  toast.info("Log in to Nexus Mods first, then start the download again");
  return true;
}

/** Opens the next Nexus file's page; the click there comes back as an nxm:// link. */
let opening: Promise<void> | null = null;
export function nextFromNexus(show = false): Promise<void> {
  if (needsNexusLogin()) return Promise.resolve();
  // One at a time: queueing the files below fires a queue event that asks for the next page
  // too, and two overlapping calls used to open two Nexus windows.
  opening ??= openNextNexus(show).finally(() => (opening = null));
  return opening;
}

async function openNextNexus(show: boolean) {
  // make sure the Nexus files are in the queue, so their clicks have somewhere to go
  if (nexusPending().some((p) => !gameJobs().some((j) => j.key === p.key))) await runSync(true);
  const next = gameJobs().find((j) => j.status === "waiting" && j.manualUrl);
  if (!next?.manualUrl) {
    if (store.nexusAssist) toast.success("Everything from Nexus is in");
    stopNexus();
    return;
  }
  store.nexusAssist = true;
  assisting = next.key;
  try {
    await api.nexusBrowse(next.manualUrl, next.name, show);
  } catch (e) {
    toast.error(errorText(e));
    stopNexus();
  }
}

/** As soon as the clicked file is queued (its download runs in the background), show the next page. */
let advancing = false;
function followNexus() {
  // Not logged in: wait for an explicit click (which routes to Settings), never pop it up unasked.
  if (advancing || !store.settings?.nexusLoggedIn) return;
  const j = gameJobs().find((x) => x.key === assisting);
  if (j && j.status === "waiting") return;
  advancing = true;
  void nextFromNexus().finally(() => (advancing = false));
}

export function stopNexus() {
  const was = store.nexusAssist;
  store.nexusAssist = false;
  assisting = null;
  if (was) void api.nexusBrowseClose().catch(() => {});
}

const VERBS: Record<string, string> = { add: "added", update: "updated", remove: "removed", import: "imported" };

let started = false;
export async function startEvents() {
  if (started) return;
  started = true;
  await listen<boolean>("nexus-login", (e) => {
    if (store.settings) store.settings.nexusLoggedIn = e.payload;
    if (e.payload) toast.success("Logged in to Nexus Mods");
  });
  await listen<Job[]>("queue", (e) => onQueue(e.payload));
  onQueue(await api.queue().catch(() => []));
  await listen<boolean>("live", (e) => (store.live = e.payload));
  // a "Mod Manager Download" click on Nexus: install that file, then open the next one
  await listen<{ modId: string; fileId: string }>("nexus-link", async (e) => {
    const m = store.overview?.mods.find((x) => x.source === "nexus" && x.sourceId === e.payload.modId);
    if (m && m.fileRef !== e.payload.fileId) {
      toast.error(`That's a different file of ${m.name} than the one on the list; use the page Mods Manager opened`);
      return;
    }
    toast.info(m ? `Got ${m.name} from Nexus, installing…` : "Got a file from Nexus");
  });
  // the in-app Nexus window was closed: stop walking through the files
  await listen("nexus-closed", () => {
    store.nexusAssist = false;
    assisting = null;
  });
  await listen<string>("nexus-error", (e) => toast.error(e.payload));
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
    if (here && store.settings?.autoSync && store.overview?.plan.length) void runSync(true);
  });
  // presence: tell the others we're here, see who else is
  setInterval(() => void refresh(), 60_000);
}
