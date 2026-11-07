<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { open } from "@tauri-apps/plugin-dialog";
import { ArrowLeft, Box, ChevronDown, FolderOpen, Gamepad2, Loader2, ScanSearch, Trash2 } from "lucide-vue-next";
import AppDialog from "./AppDialog.vue";
import { api, LOADER_LABEL, type GamePaths, type GameView, type Loader, type NewGame } from "@/lib/api";
import { loadSettings, refresh, selectGame } from "@/lib/store";
import { errorText, toast } from "@/lib/toast";

/**
 * Add a game (from a preset or by hand), edit one, or (`pathsOnly`) just say where an existing
 * game lives on this computer, e.g. after joining someone's list.
 */
const props = defineProps<{ game?: GameView; pathsOnly?: boolean }>();
const emit = defineEmits<{ close: [] }>();

const BLANK: NewGame = { name: "", modioId: 0, modioSlug: "", thunderstore: "", loader: "none", steamDir: "", contentHint: "" };
const presets = ref<NewGame[]>([]);
const step = ref<"pick" | "form">(props.game ? "form" : "pick");
const g = ref<NewGame>(props.game ? { ...props.game } : { ...BLANK });
const paths = ref<GamePaths>(props.game ? { ...props.game.paths } : { contentDir: "", gameDir: "" });
const advanced = ref(false);
const detecting = ref(false);
const saving = ref(false);
const confirmDelete = ref(false);
const error = ref("");

const title = computed(() => (props.pathsOnly ? `${props.game?.name} on this computer` : props.game ? `Edit ${props.game.name}` : "Add a game"));
const needsGameDir = computed(() => g.value.loader !== "none");

onMounted(async () => {
  presets.value = await api.presets().catch(() => []);
  if (props.game && !paths.value.contentDir && !paths.value.gameDir) await detect(true);
});

async function pick(p: NewGame | null) {
  g.value = p ? { ...p } : { ...BLANK };
  advanced.value = !p;
  step.value = "form";
  paths.value = { contentDir: "", gameDir: "" };
  if (p) await detect(true);
}

async function detect(quiet = false) {
  detecting.value = true;
  try {
    const found = await api.detectFolders(g.value.steamDir, g.value.contentHint);
    if (found.gameDir) paths.value.gameDir = found.gameDir;
    if (found.contentDir) paths.value.contentDir = found.contentDir;
    if (!quiet && !found.gameDir && !found.contentDir) toast.info(`Couldn't find ${g.value.name || "the game"} on this computer; choose the folders`);
  } finally {
    detecting.value = false;
  }
}

async function browse(key: keyof GamePaths) {
  const dir = await open({ directory: true, defaultPath: paths.value[key] || undefined });
  if (typeof dir === "string") paths.value[key] = dir;
}

async function save() {
  error.value = "";
  if (!props.pathsOnly && !g.value.name.trim()) return (error.value = "Give the game a name");
  if (!paths.value.contentDir.trim() && !paths.value.gameDir.trim()) return (error.value = "Choose at least one folder");
  if (needsGameDir.value && !paths.value.gameDir.trim()) return (error.value = `${LOADER_LABEL[g.value.loader]} mods need the game folder`);
  saving.value = true;
  try {
    if (props.pathsOnly && props.game) await api.setGamePaths(props.game.id, paths.value);
    else if (props.game) await api.updateGame(props.game.id, g.value, paths.value);
    else {
      const id = await api.createGame(g.value, paths.value);
      toast.success(`${g.value.name} is shared: everyone with the database sees it`);
      await loadSettings();
      await selectGame(id);
    }
    await loadSettings();
    await refresh();
    emit("close");
  } catch (e) {
    error.value = errorText(e);
  } finally {
    saving.value = false;
  }
}

async function remove() {
  if (!props.game) return;
  try {
    await api.deleteGame(props.game.id);
    toast.success(`${props.game.name} removed for everyone`);
    await loadSettings();
    await refresh();
    emit("close");
  } catch (e) {
    error.value = errorText(e);
  }
}

const LOADERS: { id: Loader; hint: string }[] = [
  { id: "none", hint: "Mods are just folders (Blade & Sorcery, most mod.io games)" },
  { id: "melonloader", hint: "Code mods are MelonLoader DLLs (BONELAB, BONEWORKS)" },
  { id: "bepinex", hint: "Code mods are BepInEx plugins (most Thunderstore games)" },
];
</script>

<template>
  <AppDialog :title="title" :width="600" @close="emit('close')">
    <!-- 1: pick a known game, or describe another one -->
    <template v-if="step === 'pick'">
      <p class="hint">The game and its mod list are shared with everyone on the database. The folders below stay on this computer.</p>
      <div class="preset-grid">
        <button v-for="p in presets" :key="p.name" class="card card-hover option preset" @click="pick(p)">
          <span class="icon"><Gamepad2 /></span>
          <span class="stack" style="gap: 2px; text-align: left">
            <strong>{{ p.name }}</strong>
            <span class="hint">{{ [p.modioSlug && "mod.io", p.thunderstore && "Thunderstore"].filter(Boolean).join(" · ") }} · {{ LOADER_LABEL[p.loader] }}</span>
          </span>
        </button>
        <button class="card card-hover option preset" @click="pick(null)">
          <span class="icon"><Box /></span>
          <span class="stack" style="gap: 2px; text-align: left">
            <strong>Another game</strong>
            <span class="hint">Any game on mod.io or Thunderstore, or with mods from links</span>
          </span>
        </button>
      </div>
    </template>

    <template v-else>
      <button v-if="!game" class="btn btn-ghost btn-sm" style="align-self: flex-start" @click="step = 'pick'"><ArrowLeft /> Back</button>

      <template v-if="!pathsOnly">
        <span class="section-title">The game (for everyone)</span>
        <div class="field">
          <label for="g-name">Name</label>
          <input id="g-name" v-model="g.name" class="input" placeholder="Blade & Sorcery" />
        </div>
        <div class="row" style="gap: 12px; align-items: flex-start">
          <div class="field" style="flex: 1">
            <label for="g-modio">mod.io game <span class="hint">(optional)</span></label>
            <input id="g-modio" v-model="g.modioSlug" class="input mono" placeholder="bladeandsorcery or mod.io/g/…" @input="g.modioId = 0" />
          </div>
          <div class="field" style="flex: 1">
            <label for="g-ts">Thunderstore community <span class="hint">(optional)</span></label>
            <input id="g-ts" v-model="g.thunderstore" class="input mono" placeholder="lethal-company or thunderstore.io/c/…" />
          </div>
        </div>
        <div class="field">
          <label>Code mods</label>
          <div class="choices">
            <button v-for="l in LOADERS" :key="l.id" type="button" class="choice" :class="{ on: g.loader === l.id }" :title="l.hint" @click="g.loader = l.id">{{ LOADER_LABEL[l.id] }}</button>
          </div>
          <span class="hint">{{ LOADERS.find((l) => l.id === g.loader)?.hint }}</span>
        </div>
        <button type="button" class="btn btn-ghost btn-sm" style="align-self: flex-start" @click="advanced = !advanced">
          <ChevronDown :style="{ transform: advanced ? 'rotate(180deg)' : '' }" /> Finding it on each computer
        </button>
        <div v-if="advanced" class="row" style="gap: 12px; align-items: flex-start">
          <div class="field" style="flex: 1">
            <label for="g-steam">Steam folder name</label>
            <input id="g-steam" v-model="g.steamDir" class="input mono" placeholder="Blade & Sorcery" />
            <span class="hint">Its folder in steamapps\common.</span>
          </div>
          <div class="field" style="flex: 1">
            <label for="g-hint">Usual mods folder</label>
            <input id="g-hint" v-model="g.contentHint" class="input mono" placeholder="%GAME%/…/Mods" />
            <span class="hint">%GAME%, %LOCALLOW% and %DOCUMENTS% work.</span>
          </div>
        </div>
      </template>

      <span class="section-title">On this computer</span>
      <div class="field">
        <label for="g-content">Mods folder <span class="hint">(content mods go here)</span></label>
        <div class="row" style="gap: 8px">
          <input id="g-content" v-model="paths.contentDir" class="input mono" placeholder="Where the game loads mod folders from" />
          <button type="button" class="btn btn-icon" title="Browse" aria-label="Browse" @click="browse('contentDir')"><FolderOpen /></button>
        </div>
      </div>
      <div class="field">
        <label for="g-dir">Game folder <span class="hint">{{ needsGameDir ? `(${LOADER_LABEL[g.loader]} lives here)` : "(optional)" }}</span></label>
        <div class="row" style="gap: 8px">
          <input id="g-dir" v-model="paths.gameDir" class="input mono" placeholder="…\steamapps\common\<game>" />
          <button type="button" class="btn btn-icon" title="Browse" aria-label="Browse" @click="browse('gameDir')"><FolderOpen /></button>
        </div>
      </div>
      <button v-if="g.steamDir || g.contentHint" type="button" class="btn btn-sm" style="align-self: flex-start" :disabled="detecting" @click="detect()">
        <Loader2 v-if="detecting" class="spin" /><ScanSearch v-else /> Find automatically
      </button>

      <p v-if="error" class="error-text selectable">{{ error }}</p>
      <div v-if="confirmDelete" class="inset stack" style="gap: 10px; padding: 14px">
        <strong>Remove {{ game?.name }} for everyone?</strong>
        <span class="hint">Its mod list and activity are deleted from the shared database. Installed files stay on every computer.</span>
        <div class="row" style="gap: 8px; justify-content: flex-end">
          <button class="btn" @click="confirmDelete = false">Cancel</button>
          <button class="btn btn-danger" @click="remove"><Trash2 /> Remove</button>
        </div>
      </div>
      <div v-else class="dialog-footer">
        <button v-if="game && !pathsOnly" class="btn btn-danger" style="margin-right: auto" @click="confirmDelete = true"><Trash2 /> Remove game</button>
        <button class="btn" @click="emit('close')">Cancel</button>
        <button class="btn btn-primary" :disabled="saving" @click="save">
          <Loader2 v-if="saving" class="spin" /> {{ game ? "Save" : "Add game" }}
        </button>
      </div>
    </template>
  </AppDialog>
</template>
