import { reactive } from "vue";
import { getVersion } from "@tauri-apps/api/app";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { store } from "./store";
import { errorText, toast } from "./toast";

/**
 * App updates from GitHub releases (signed: the updater refuses anything else).
 *
 * Right after start a new version installs on its own (nothing is happening yet). While the app
 * is open it checks every 30 minutes and when the window comes back into focus; the top bar
 * then shows "Restart to update". A running sync is never cut off: the update waits for it.
 */
export const updater = reactive({
  current: "",
  available: null as null | { version: string; notes: string },
  checking: false,
  installing: false,
  /** 0–1 while downloading */
  progress: 0,
});

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
let pending: Update | null = null;
let lastCheck = 0;

export async function checkForUpdate(manual = false) {
  if (updater.checking || updater.installing) return;
  updater.checking = true;
  lastCheck = Date.now();
  try {
    updater.current ||= await getVersion();
    pending = await check();
    updater.available = pending ? { version: pending.version, notes: pending.body ?? "" } : null;
    if (manual && !pending) toast.success(`Mods Manager is up to date (${updater.current})`);
  } catch (e) {
    if (manual) toast.error(`Couldn't check for updates: ${errorText(e)}`);
  } finally {
    updater.checking = false;
  }
}

export async function installUpdate() {
  if (!pending || updater.installing) return;
  updater.installing = true;
  try {
    while (store.syncing) await sleep(500);
    let total = 0;
    let done = 0;
    await pending.downloadAndInstall((e) => {
      if (e.event === "Started") total = e.data.contentLength ?? 0;
      else if (e.event === "Progress") {
        done += e.data.chunkLength;
        updater.progress = total ? done / total : 0;
      }
    });
    await relaunch();
  } catch (e) {
    toast.error(`Update failed: ${errorText(e)}`);
    updater.installing = false;
  }
}

export async function startUpdateChecks() {
  updater.current = await getVersion().catch(() => "");
  await checkForUpdate();
  if (pending) {
    toast.info(`Updating to ${pending.version}…`);
    await installUpdate();
  }
  setInterval(() => void checkForUpdate(), 30 * 60_000);
  window.addEventListener("focus", () => {
    if (Date.now() - lastCheck > 10 * 60_000) void checkForUpdate();
  });
}
