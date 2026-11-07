import { reactive } from "vue";
import { api, type Settings } from "./api";
import { refresh, store } from "./store";

export type SettingsSection = "profile" | "shared" | "modio" | "folders" | "sync" | "appearance" | "maintenance";

export const settingsDialog = reactive({ open: false, section: "profile" as SettingsSection });

export function openSettings(section: SettingsSection = "profile") {
  settingsDialog.section = section;
  settingsDialog.open = true;
}

/** Saves right away, like Tessera's settings: there's no Save button. */
export async function saveSettings(patch: Partial<Settings>) {
  if (!store.settings) return;
  const next = { ...store.settings, ...patch };
  await api.saveSettings(next);
  store.settings = next;
  await refresh();
}
