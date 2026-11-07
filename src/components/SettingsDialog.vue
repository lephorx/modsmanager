<script setup lang="ts">
import { onMounted, onUnmounted, ref, watch, type Component } from "vue";
import { Check, CircleCheck, Database, FolderCog, FolderOpen, Gamepad2, KeyRound, Loader2, Paintbrush, Pencil, Plus, RefreshCw, Link2, Crown, User, UserPlus, Wrench, X, Zap } from "lucide-vue-next";
import markUrl from "@/assets/mark.svg";
import SecretInput from "./SecretInput.vue";
import UnmanagedDialog from "./UnmanagedDialog.vue";
import InviteDialog from "./InviteDialog.vue";
import { api, DB_PLACEHOLDER, LOADER_LABEL, type NexusAccount, type Settings } from "@/lib/api";
import { saveSettings, settingsDialog, type SettingsSection } from "@/lib/settingsDialog";
import { gameDialog } from "@/lib/dialogs";
import { currentDesign, setDesign, type Design } from "@/lib/design";
import { currentGame, hasPaths, refresh, store } from "@/lib/store";
import { errorText, toast } from "@/lib/toast";

const groups: { title: string; items: { id: SettingsSection; label: string; icon: Component }[] }[] = [
  { title: "You", items: [{ id: "profile", label: "Profile", icon: User }] },
  {
    title: "Sharing",
    items: [
      { id: "shared", label: "Shared database", icon: Database },
      { id: "games", label: "Games", icon: Gamepad2 },
      { id: "sync", label: "Syncing", icon: Zap },
    ],
  },
  {
    title: "Sources",
    items: [
      { id: "modio", label: "mod.io", icon: KeyRound },
      { id: "nexus", label: "Nexus Mods", icon: Link2 },
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

/** A local copy: fields save when you leave them. */
const s = ref<Settings>({ ...store.settings! });
const version = ref("");
const design = ref<Design>(currentDesign());
const testing = ref(false);
const tested = ref<"" | "ok" | string>("");
const importing = ref(false);
const reviewing = ref(false);
const inviting = ref(false);
const game = () => store.games?.find((g) => g.id === currentGame()) ?? null;

const nexus = ref<NexusAccount | null>(null);
const nexusChecking = ref(false);
const nexusError = ref("");
const nxmLink = ref("");
// bound, not inline: vue-tsc mis-parses the // in a static attribute
const NXM_PLACEHOLDER = "nxm://…";
async function checkNexus() {
  nexusError.value = "";
  nexus.value = null;
  if (!store.settings?.nexusApiKey.trim()) return;
  nexusChecking.value = true;
  try {
    nexus.value = await api.nexusAccount();
  } catch (e) {
    nexusError.value = errorText(e);
  } finally {
    nexusChecking.value = false;
  }
}
watch(
  () => [settingsDialog.section, store.settings?.nexusApiKey],
  () => settingsDialog.section === "nexus" && void checkNexus(),
  { immediate: true },
);
async function pasteNxm() {
  try {
    await api.nexusLink(nxmLink.value.trim());
    nxmLink.value = "";
    toast.success("Got it; the sync installs it now");
  } catch (e) {
    toast.error(errorText(e));
  }
}

const close = () => (settingsDialog.open = false);
function onKey(e: KeyboardEvent) {
  if (e.key === "Escape" && !reviewing.value && !inviting.value) close();
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

function pickDesign(d: Design) {
  design.value = d;
  setDesign(d);
}

/** Dialogs from the top level, over Settings. */
function openGame(kind: "add" | "edit" | "paths", id?: number) {
  const g = store.games?.find((x) => x.id === id) ?? null;
  close();
  if (kind === "add") gameDialog.add = true;
  else if (kind === "edit") gameDialog.edit = g;
  else gameDialog.paths = g;
}

async function importLegacy() {
  const id = currentGame();
  if (id == null) return;
  importing.value = true;
  try {
    const [n, failed] = await api.importLegacy(id);
    if (failed.length) toast.error(`Imported ${n}; ${failed.length} failed: ${failed.join("; ")}`);
    else toast.success(n ? `Imported ${n} mods from the old manager` : "Everything from the old manager is already on the list");
    await refresh();
  } catch (e) {
    toast.error(errorText(e));
  } finally {
    importing.value = false;
  }
}

function openFolder(which: "content" | "game") {
  const id = currentGame();
  if (id != null) api.openFolder(id, which).catch((e) => toast.error(errorText(e)));
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
                <p class="set-hint">Everyone sees it next to what you add, update or remove.</p>
                <input v-model="s.name" class="input" @blur="commit('name')" @keydown.enter="commit('name')" />
              </div>
            </template>

            <template v-else-if="settingsDialog.section === 'shared'">
              <div class="set-block">
                <h3 class="set-title">Shared database</h3>
                <p class="set-hint">The Postgres database that holds every game's list. Everyone uses the same address. Mods Manager creates its own tables (mm_…).</p>
                <div class="row" style="gap: 8px">
                  <SecretInput v-model="s.databaseUrl" :placeholder="DB_PLACEHOLDER" style="flex: 1" @done="commit('databaseUrl')" @update:model-value="tested = ''" />
                  <button class="btn" :disabled="testing || !s.databaseUrl.trim()" @click="test">
                    <Loader2 v-if="testing" class="spin" /><CircleCheck v-else-if="tested === 'ok'" style="color: var(--status-green)" /> Test
                  </button>
                </div>
                <p v-if="tested && tested !== 'ok'" class="error-text selectable">{{ tested }}</p>
                <p class="set-hint">
                  <span class="live" :class="{ on: store.live }"><span class="run-dot" />{{ store.live ? "Live updates connected" : "Live updates not connected" }}</span>
                </p>
              </div>
              <div class="set-block">
                <h3 class="set-title">Invite people</h3>
                <p class="set-hint">An invite code sets up someone's app in one paste: the database and the game you're on. Anyone with it can join.</p>
                <button class="btn" style="align-self: flex-start" @click="inviting = true"><UserPlus /> Invite someone</button>
              </div>
            </template>

            <template v-else-if="settingsDialog.section === 'games'">
              <div class="set-block">
                <h3 class="set-title">Games</h3>
                <p class="set-hint">Each game has its own shared list. Folders are per computer.</p>
                <div v-if="store.games?.length" class="set-table">
                  <div v-for="g in store.games" :key="g.id" class="set-row">
                    <Gamepad2 :size="18" class="muted" />
                    <div class="stack" style="gap: 2px; flex: 1; min-width: 0">
                      <strong class="ellipsis">{{ g.name }}</strong>
                      <span class="hint ellipsis">
                        {{ g.mods }} mods · {{ LOADER_LABEL[g.loader] }}
                        <span v-if="!hasPaths(g.id)" style="color: var(--status-orange)"> · folders not set here</span>
                      </span>
                    </div>
                    <button class="btn btn-sm" title="Folders on this computer" @click="openGame('paths', g.id)"><FolderCog /> Folders</button>
                    <button class="btn btn-sm btn-icon" :title="`Edit ${g.name}`" :aria-label="`Edit ${g.name}`" @click="openGame('edit', g.id)"><Pencil /></button>
                  </div>
                </div>
                <button class="btn" style="align-self: flex-start" @click="openGame('add')"><Plus /> Add a game</button>
              </div>
            </template>

            <template v-else-if="settingsDialog.section === 'sync'">
              <div class="set-block">
                <div class="set-table">
                  <label class="set-row">
                    <div class="stack" style="gap: 2px; flex: 1">
                      <strong>Sync on its own</strong>
                      <span class="hint">When someone adds, updates or removes a mod in the game you're on, install the change right away.</span>
                    </div>
                    <span class="switch"><input v-model="s.autoSync" type="checkbox" @change="commit('autoSync')" /><span /></span>
                  </label>
                </div>
              </div>
            </template>

            <template v-else-if="settingsDialog.section === 'modio'">
              <div class="set-block">
                <h3 class="set-title">API key</h3>
                <p class="set-hint">For searching and reading mods on mod.io, for every game. Get one on mod.io under Account → API access.</p>
                <SecretInput v-model="s.modioApiKey" @done="commit('modioApiKey')" />
              </div>
              <div class="set-block">
                <h3 class="set-title">OAuth token <span class="hint">(optional)</span></h3>
                <p class="set-hint">Only needed if downloads fail with "unauthorized": some files need a signed-in mod.io user. Create one under API access → OAuth 2 tokens.</p>
                <SecretInput v-model="s.modioToken" @done="commit('modioToken')" />
              </div>
            </template>

            <template v-else-if="settingsDialog.section === 'nexus'">
              <div class="set-block">
                <h3 class="set-title">API key</h3>
                <p class="set-hint">For Nexus Mods games. Find it on nexusmods.com → your avatar → Site preferences → API Keys (Personal API Key, at the bottom).</p>
                <SecretInput v-model="s.nexusApiKey" @done="commit('nexusApiKey')" />
                <p v-if="nexusChecking" class="set-hint row" style="gap: 6px"><Loader2 class="spin" :size="14" /> Checking the key…</p>
                <p v-else-if="nexusError" class="error-text selectable">{{ nexusError }}</p>
                <p v-else-if="nexus" class="set-hint row" style="gap: 6px">
                  <Crown v-if="nexus.premium" :size="14" style="color: var(--status-orange)" /><CircleCheck v-else :size="14" style="color: var(--status-green)" />
                  <span>
                    <strong>{{ nexus.name }}</strong> ·
                    {{ nexus.premium ? "Premium: Nexus mods sync on their own." : "Free account: each Nexus mod needs one click on the website per computer. Sync opens the exact file; you click Slow download." }}
                  </span>
                </p>
              </div>
              <div class="set-block">
                <div class="set-table">
                  <label class="set-row">
                    <div class="stack" style="gap: 2px; flex: 1">
                      <strong>Open "Mod Manager Download" links</strong>
                      <span class="hint">Nexus hands the click to Mods Manager (nxm:// links). Turn off if Vortex or Mod Organizer should get them instead.</span>
                    </div>
                    <span class="switch">
                      <input type="checkbox" :checked="!s.nexusIgnoreLinks" @change="(s.nexusIgnoreLinks = !($event.target as HTMLInputElement).checked), commit('nexusIgnoreLinks')" /><span />
                    </span>
                  </label>
                </div>
              </div>
              <div class="set-block">
                <h3 class="set-title">Paste a download link</h3>
                <p class="set-hint">If the browser doesn't open Mods Manager: right-click "Mod Manager Download" on the file, copy the link (nxm://…), paste it here.</p>
                <div class="row" style="gap: 8px">
                  <SecretInput v-model="nxmLink" :placeholder="NXM_PLACEHOLDER" style="flex: 1" />
                  <button class="btn" :disabled="!nxmLink.trim()" @click="pasteNxm">Use link</button>
                </div>
              </div>
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
              <template v-if="game()">
                <div class="set-block">
                  <h3 class="set-title">Files not on {{ game()!.name }}'s list</h3>
                  <p class="set-hint">Mods in its folders that the list doesn't know about. The others don't have them.</p>
                  <button class="btn" style="align-self: flex-start" @click="reviewing = true">Review ({{ store.overview?.unmanaged ?? 0 }})</button>
                </div>
                <div class="set-block">
                  <h3 class="set-title">Folders</h3>
                  <div class="row" style="gap: 8px">
                    <button class="btn" @click="openFolder('content')"><FolderOpen /> Mods folder</button>
                    <button class="btn" @click="openFolder('game')"><FolderOpen /> Game folder</button>
                  </div>
                </div>
                <div class="set-block">
                  <h3 class="set-title">Import from the old manager</h3>
                  <p class="set-hint">
                    Copies the mods from the old Mod.io-Mods-Manager tables in this database onto {{ game()!.name }}'s list, pinned to today's files. Run it once.
                  </p>
                  <button class="btn" style="align-self: flex-start" :disabled="importing" @click="importLegacy">
                    <Loader2 v-if="importing" class="spin" /><RefreshCw v-else /> Import into {{ game()!.name }}
                  </button>
                </div>
              </template>
              <p v-else class="set-hint">Add a game first.</p>
            </template>
          </section>
        </div>
      </div>
    </div>
    <UnmanagedDialog v-if="reviewing && game()" :game="game()!.id" @close="(reviewing = false), refresh()" />
    <InviteDialog v-if="inviting" @close="inviting = false" />
  </Teleport>
</template>
