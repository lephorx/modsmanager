<script setup lang="ts">
import { onMounted, onUnmounted, ref, type Component } from "vue";
import { Check, CircleCheck, Database, FolderOpen, KeyRound, Loader2, Paintbrush, RefreshCw, ScanSearch, User, Wrench, X, Zap } from "lucide-vue-next";
import { open } from "@tauri-apps/plugin-dialog";
import markUrl from "@/assets/mark.svg";
import { api, type Settings } from "@/lib/api";
import { saveSettings, settingsDialog, type SettingsSection } from "@/lib/settingsDialog";
import { currentDesign, setDesign, type Design } from "@/lib/design";
import { refresh, store } from "@/lib/store";
import { errorText, toast } from "@/lib/toast";
import UnmanagedDialog from "./UnmanagedDialog.vue";

const groups: { title: string; items: { id: SettingsSection; label: string; icon: Component }[] }[] = [
  { title: "You", items: [{ id: "profile", label: "Profile", icon: User }] },
  {
    title: "Sharing",
    items: [
      { id: "shared", label: "Shared list", icon: Database },
      { id: "sync", label: "Syncing", icon: Zap },
    ],
  },
  {
    title: "BONELAB",
    items: [
      { id: "modio", label: "mod.io", icon: KeyRound },
      { id: "folders", label: "Folders", icon: FolderOpen },
    ],
  },
  {
    title: "App",
    items: [
      { id: "appearance", label: "Appearance", icon: Paintbrush },
      { id: "maintenance", label: "Maintenance", icon: Wrench },
    ],
  },
];

/** A local copy: text fields save when you leave them. */
const s = ref<Settings>({ ...store.settings! });
const version = ref("");
const design = ref<Design>(currentDesign());
const testing = ref(false);
const tested = ref<"" | "ok" | string>("");
const importing = ref(false);
const reviewing = ref(false);

const close = () => (settingsDialog.open = false);
function onKey(e: KeyboardEvent) {
  if (e.key === "Escape" && !reviewing.value) close();
}
onMounted(async () => {
  window.addEventListener("keydown", onKey);
  version.value = await api.version().catch(() => "");
});
onUnmounted(() => window.removeEventListener("keydown", onKey));

async function save(patch: Partial<Settings>) {
  try {
    await saveSettings(patch);
  } catch (e) {
    toast.error(errorText(e));
  }
}

function commit<K extends keyof Settings>(key: K) {
  if (store.settings && s.value[key] !== store.settings[key]) void save({ [key]: s.value[key] } as Partial<Settings>);
}

async function test() {
  testing.value = true;
  tested.value = "";
  try {
    await api.testConnection(s.value.databaseUrl);
    tested.value = "ok";
  } catch (e) {
    tested.value = errorText(e);
  } finally {
    testing.value = false;
  }
}

async function browse(key: "modsDir" | "gameDir") {
  const dir = await open({ directory: true, defaultPath: s.value[key] || undefined });
  if (typeof dir === "string") {
    s.value[key] = dir;
    commit(key);
  }
}

async function detect() {
  const found = await api.detectFolders();
  const patch: Partial<Settings> = {};
  if (found.modsDir) patch.modsDir = s.value.modsDir = found.modsDir;
  if (found.gameDir) patch.gameDir = s.value.gameDir = found.gameDir;
  if (!found.modsDir && !found.gameDir) return toast.info("Couldn't find BONELAB on this computer");
  await save(patch);
  toast.success("Found BONELAB's folders");
}

function pickDesign(d: Design) {
  design.value = d;
  setDesign(d);
}

async function importLegacy() {
  importing.value = true;
  try {
    const [n, failed] = await api.importLegacy();
    if (failed.length) toast.error(`Imported ${n}; ${failed.length} failed: ${failed.join("; ")}`);
    else toast.success(n ? `Imported ${n} mods from the old manager` : "Everything from the old manager is already on the list");
    await refresh();
  } catch (e) {
    toast.error(errorText(e));
  } finally {
    importing.value = false;
  }
}
</script>

<template>
  <Teleport to="body">
    <div class="backdrop" @mousedown.self="close">
      <div class="dialog settings-dialog" role="dialog" aria-label="Settings">
        <div class="dialog-head">
          <h2>Settings</h2>
          <button class="dialog-close" aria-label="Close" title="Close" @click="close"><X /></button>
        </div>
        <div class="settings-body">
          <nav class="settings-nav">
            <div class="settings-nav-list">
              <template v-for="g in groups" :key="g.title">
                <span class="settings-group">{{ g.title }}</span>
                <button
                  v-for="i in g.items"
                  :key="i.id"
                  class="settings-link"
                  :class="{ on: settingsDialog.section === i.id }"
                  :aria-current="settingsDialog.section === i.id ? 'page' : undefined"
                  @click="settingsDialog.section = i.id"
                >
                  <component :is="i.icon" :size="18" /> {{ i.label }}
                </button>
              </template>
            </div>
            <div class="settings-about">
              <img :src="markUrl" alt="" width="28" height="28" />
              <span>Mods Manager {{ version }}</span>
            </div>
          </nav>

          <section class="settings-content">
            <template v-if="settingsDialog.section === 'profile'">
              <div class="set-block">
                <h3 class="set-title">Your name</h3>
                <p class="set-hint">Your friend sees it next to everything you add, update or remove.</p>
                <input v-model="s.name" class="input" @blur="commit('name')" @keydown.enter="commit('name')" />
              </div>
            </template>

            <template v-else-if="settingsDialog.section === 'shared'">
              <div class="set-block">
                <h3 class="set-title">Shared database</h3>
                <p class="set-hint">The Postgres database that holds the list. Both of you use the same address. Mods Manager creates its own tables (mm_mods, mm_events, mm_members).</p>
                <div class="row" style="gap: 8px">
                  <input v-model="s.databaseUrl" class="input mono" placeholder="postgres://user:password@host:5432/mods" @blur="commit('databaseUrl')" @input="tested = ''" />
                  <button class="btn" :disabled="testing || !s.databaseUrl.trim()" @click="test">
                    <Loader2 v-if="testing" class="spin" /><CircleCheck v-else-if="tested === 'ok'" style="color: var(--status-green)" /> Test
                  </button>
                </div>
                <p v-if="tested && tested !== 'ok'" class="error-text selectable">{{ tested }}</p>
                <p class="set-hint">
                  <span class="live" :class="{ on: store.live }"><span class="run-dot" />{{ store.live ? "Live updates connected" : "Live updates not connected" }}</span>
                </p>
              </div>
            </template>

            <template v-else-if="settingsDialog.section === 'sync'">
              <div class="set-block">
                <div class="set-table">
                  <label class="set-row">
                    <div class="stack" style="gap: 2px; flex: 1">
                      <strong>Sync on its own</strong>
                      <span class="hint">When your friend adds, updates or removes a mod while the app is open, install the change right away.</span>
                    </div>
                    <span class="switch"><input v-model="s.autoSync" type="checkbox" @change="commit('autoSync')" /><span /></span>
                  </label>
                </div>
              </div>
            </template>

            <template v-else-if="settingsDialog.section === 'modio'">
              <div class="set-block">
                <h3 class="set-title">API key</h3>
                <p class="set-hint">For searching and reading mods. Get one on mod.io under Account → API access.</p>
                <input v-model="s.modioApiKey" class="input mono" @blur="commit('modioApiKey')" />
              </div>
              <div class="set-block">
                <h3 class="set-title">OAuth token <span class="hint">(optional)</span></h3>
                <p class="set-hint">Only needed if downloads fail with "unauthorized": some files need a signed-in mod.io user. Create one under API access → OAuth 2 tokens.</p>
                <input v-model="s.modioToken" class="input mono" type="password" @blur="commit('modioToken')" />
              </div>
              <div class="set-block">
                <h3 class="set-title">Game id</h3>
                <p class="set-hint">BONELAB is 3809.</p>
                <input v-model.number="s.gameId" class="input num" type="number" style="width: 140px" @blur="commit('gameId')" />
              </div>
            </template>

            <template v-else-if="settingsDialog.section === 'folders'">
              <div class="set-block">
                <h3 class="set-title">BONELAB Mods folder</h3>
                <p class="set-hint">Where content mods (pallets) go.</p>
                <div class="row" style="gap: 8px">
                  <input v-model="s.modsDir" class="input mono" @blur="commit('modsDir')" />
                  <button class="btn btn-icon" title="Browse" aria-label="Browse" @click="browse('modsDir')"><FolderOpen /></button>
                </div>
              </div>
              <div class="set-block">
                <h3 class="set-title">BONELAB game folder</h3>
                <p class="set-hint">Where MelonLoader and code mods live (it has Mods, Plugins, UserLibs and UserData).</p>
                <div class="row" style="gap: 8px">
                  <input v-model="s.gameDir" class="input mono" @blur="commit('gameDir')" />
                  <button class="btn btn-icon" title="Browse" aria-label="Browse" @click="browse('gameDir')"><FolderOpen /></button>
                </div>
                <p class="set-hint">
                  MelonLoader:
                  <strong :style="{ color: store.overview?.melon ? 'var(--status-green)' : 'var(--status-orange)' }">{{ store.overview?.melon ? "installed" : "not found" }}</strong>
                </p>
              </div>
              <button class="btn" style="align-self: flex-start" @click="detect"><ScanSearch /> Find BONELAB automatically</button>
            </template>

            <template v-else-if="settingsDialog.section === 'appearance'">
              <div class="set-block">
                <h3 class="set-title">Design</h3>
                <p class="set-hint">The same two looks as Tessera.</p>
                <div class="row" style="gap: 12px">
                  <button
                    v-for="d in (['modern', 'classic'] as Design[])"
                    :key="d"
                    type="button"
                    class="card card-hover option"
                    :class="{ selected: design === d }"
                    style="flex: 1; flex-direction: column; align-items: stretch; gap: 10px"
                    @click="pickDesign(d)"
                  >
                    <span
                      :style="{
                        height: '90px',
                        background: d === 'modern' ? 'linear-gradient(160deg,#2b2e35,#16181d)' : '#09090b',
                        borderRadius: d === 'modern' ? '12px' : '0',
                        border: d === 'classic' ? '1px solid rgb(255 255 255 / .1)' : 'none',
                        display: 'flex',
                        gap: '8px',
                        padding: '14px',
                      }"
                    >
                      <span :style="{ width: '34%', background: d === 'modern' ? '#212329' : '#18181b', borderRadius: d === 'modern' ? '8px' : '0' }" />
                      <span :style="{ flex: 1, background: d === 'modern' ? '#212329' : '#18181b', borderRadius: d === 'modern' ? '8px' : '0' }" />
                    </span>
                    <span class="row">
                      <span style="font-weight: 600">{{ d === "modern" ? "Modern" : "Classic" }}</span>
                      <span class="spacer" />
                      <Check v-if="design === d" :size="16" style="color: var(--tint)" />
                    </span>
                  </button>
                </div>
              </div>
            </template>

            <template v-else-if="settingsDialog.section === 'maintenance'">
              <div class="set-block">
                <h3 class="set-title">Files not on the list</h3>
                <p class="set-hint">Mods in your folders that the list doesn't know about. Your friend doesn't have them.</p>
                <button class="btn" style="align-self: flex-start" @click="reviewing = true">Review ({{ store.overview?.unmanaged ?? 0 }})</button>
              </div>
              <div class="set-block">
                <h3 class="set-title">Import from the old manager</h3>
                <p class="set-hint">
                  Copies the mods and code mods from the old Mod.io-Mods-Manager tables in this database onto the list, pinned to today's files. Run it once.
                </p>
                <button class="btn" style="align-self: flex-start" :disabled="importing" @click="importLegacy">
                  <Loader2 v-if="importing" class="spin" /><RefreshCw v-else /> Import
                </button>
              </div>
              <div class="set-block">
                <h3 class="set-title">Folders</h3>
                <div class="row" style="gap: 8px">
                  <button class="btn" @click="api.openFolder('mods').catch((e) => toast.error(errorText(e)))"><FolderOpen /> Mods folder</button>
                  <button class="btn" @click="api.openFolder('game').catch((e) => toast.error(errorText(e)))"><FolderOpen /> Game folder</button>
                </div>
              </div>
            </template>
          </section>
        </div>
      </div>
    </div>
    <UnmanagedDialog v-if="reviewing" @close="(reviewing = false), refresh()" />
  </Teleport>
</template>
