<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { ArrowLeft, Box, Code2, Download, Link2, Loader2, Plus, Search } from "lucide-vue-next";
import AppDialog from "./AppDialog.vue";
import { api, type Resolved, type SearchHit, type Target } from "@/lib/api";
import { refresh, runSync, store } from "@/lib/store";
import { hideImg, bytes, SOURCE_LABEL } from "@/lib/format";
import { errorText, toast } from "@/lib/toast";

const emit = defineEmits<{ close: [] }>();

const mode = ref<"search" | "link">("search");
const query = ref("");
const hits = ref<SearchHit[]>([]);
const searching = ref(false);
const searchError = ref("");

const link = ref("");
const linkName = ref("");
const linkTarget = ref<Target>("content");

const resolving = ref(false);
const resolved = ref<Resolved | null>(null);
const error = ref("");
const installNow = ref(true);
const saving = ref(false);

const isDirectLink = computed(() => {
  const s = link.value.trim();
  return /^https?:\/\//.test(s) && !s.includes("mod.io/") && !s.includes("thunderstore.io");
});

const onList = computed(() => new Set((store.overview?.mods ?? []).map((m) => `${m.source}:${m.sourceId}`)));
const already = computed(() => !!resolved.value && onList.value.has(`${resolved.value.main.source}:${resolved.value.main.sourceId}`));
const newDeps = computed(() => (resolved.value?.deps ?? []).filter((d) => !onList.value.has(`${d.source}:${d.sourceId}`)));

let timer: ReturnType<typeof setTimeout> | undefined;
let seq = 0;
async function search() {
  const mine = ++seq;
  searching.value = true;
  searchError.value = "";
  try {
    const found = await api.search(query.value);
    if (mine === seq) hits.value = found;
  } catch (e) {
    if (mine === seq) searchError.value = errorText(e);
  } finally {
    if (mine === seq) searching.value = false;
  }
}
watch(query, () => {
  clearTimeout(timer);
  timer = setTimeout(search, 300);
});
onMounted(search);

async function resolve(input: string) {
  resolving.value = true;
  error.value = "";
  resolved.value = null;
  try {
    resolved.value = await api.resolve(input, isDirectLink.value ? linkName.value : undefined, isDirectLink.value ? linkTarget.value : undefined);
  } catch (e) {
    error.value = errorText(e);
  } finally {
    resolving.value = false;
  }
}

async function add() {
  if (!resolved.value) return;
  saving.value = true;
  try {
    const n = await api.add([resolved.value.main, ...newDeps.value]);
    toast.success(n > 1 ? `${resolved.value.main.name} and ${n - 1} dependencies added for everyone` : `${resolved.value.main.name} added for everyone`);
    emit("close");
    await refresh();
    if (installNow.value) void runSync();
  } catch (e) {
    error.value = errorText(e);
  } finally {
    saving.value = false;
  }
}
</script>

<template>
  <AppDialog title="Add mod" :width="640" @close="emit('close')">
    <!-- step 2: what exactly gets pinned -->
    <template v-if="resolved || resolving">
      <button class="btn btn-ghost btn-sm" style="align-self: flex-start" @click="(resolved = null), (resolving = false), (error = '')"><ArrowLeft /> Back</button>
      <div v-if="resolving" class="inset row" style="gap: 10px; padding: 18px">
        <Loader2 class="spin" /> {{ isDirectLink ? "Downloading the file to pin its exact version…" : "Looking up the newest file…" }}
      </div>
      <template v-else-if="resolved">
        <div class="inset preview">
          <span class="mod-thumb lg">
            <Code2 v-if="resolved.main.target === 'code'" /><Box v-else />
            <img v-if="resolved.main.thumbUrl" :src="resolved.main.thumbUrl" alt="" @error="hideImg" />
          </span>
          <div class="stack" style="gap: 4px; min-width: 0; flex: 1">
            <strong style="font-size: 16px" class="ellipsis">{{ resolved.main.name }}</strong>
            <div class="row" style="gap: 6px; flex-wrap: wrap">
              <span class="chip">{{ SOURCE_LABEL[resolved.main.source] }}</span>
              <span class="chip num">{{ resolved.main.version }}</span>
              <span class="chip">{{ resolved.main.target === "code" ? "Code mod (MelonLoader)" : "Content mod" }}</span>
              <span v-if="resolved.main.fileSize" class="chip num">{{ bytes(resolved.main.fileSize) }}</span>
            </div>
            <p v-if="resolved.main.summary" class="hint clamp">{{ resolved.main.summary }}</p>
          </div>
        </div>
        <div v-if="resolved.deps.length" class="stack" style="gap: 6px">
          <span class="label">Needs {{ resolved.deps.length === 1 ? "this mod" : "these mods" }} too</span>
          <div class="inset plan">
            <div v-for="d in resolved.deps" :key="d.sourceId" class="plan-row">
              <span class="ellipsis" style="font-weight: 600">{{ d.name }}</span>
              <span class="spacer" />
              <span class="hint num">{{ onList.has(`${d.source}:${d.sourceId}`) ? "already on the list" : d.version }}</span>
            </div>
          </div>
        </div>
        <p class="hint">
          Pins exactly this file for both of you. When the mod gets a new version, nothing changes until one of you presses Update.
        </p>
        <label class="row" style="gap: 8px"><input v-model="installNow" type="checkbox" /> Install it on this computer right away</label>
      </template>
      <p v-if="error" class="error-text selectable">{{ error }}</p>
      <div class="dialog-footer">
        <button class="btn" @click="emit('close')">Cancel</button>
        <button class="btn btn-primary" :disabled="!resolved || already || saving" @click="add">
          <Loader2 v-if="saving" class="spin" /><Plus v-else />
          {{ already ? "Already on the list" : "Add for everyone" }}
        </button>
      </div>
    </template>

    <!-- step 1: find it -->
    <template v-else>
      <div v-slider class="tabs" role="tablist">
        <button :class="{ active: mode === 'search' }" role="tab" @click="mode = 'search'"><Search /> Search mod.io</button>
        <button :class="{ active: mode === 'link' }" role="tab" @click="mode = 'link'"><Link2 /> Paste a link</button>
      </div>

      <template v-if="mode === 'search'">
        <label class="input-icon">
          <Search />
          <input v-model="query" class="input" placeholder="Search BONELAB mods on mod.io" autofocus aria-label="Search mod.io" />
        </label>
        <p v-if="searchError" class="error-text selectable">{{ searchError }}</p>
        <div class="hits">
          <div v-if="searching && !hits.length" class="row hint" style="gap: 8px; padding: 12px"><Loader2 class="spin" /> Searching…</div>
          <button v-for="h in hits" :key="h.input" class="hit" @click="resolve(h.input)">
            <span class="mod-thumb"><Box /><img v-if="h.thumb" :src="h.thumb" alt="" loading="lazy" @error="hideImg" /></span>
            <span class="stack" style="gap: 2px; min-width: 0; flex: 1; text-align: left">
              <strong class="ellipsis">{{ h.name }}</strong>
              <span class="hint ellipsis">by {{ h.author }} · {{ h.downloads.toLocaleString() }} downloads</span>
            </span>
            <span v-if="onList.has(`modio:${h.id}`)" class="chip">On the list</span>
            <Download :size="16" class="muted" />
          </button>
          <p v-if="!searching && !hits.length && !searchError" class="hint" style="padding: 12px">No mods found.</p>
        </div>
      </template>

      <template v-else>
        <div class="field">
          <label for="mm-link">Link</label>
          <input id="mm-link" v-model="link" class="input" placeholder="mod.io or Thunderstore page, or a direct download link" @keydown.enter="link.trim() && resolve(link)" />
          <span class="hint">mod.io: content mods (avatars, maps, spawnables). Thunderstore: MelonLoader code mods, with their dependencies.</span>
        </div>
        <template v-if="isDirectLink">
          <div class="field">
            <label for="mm-name">Name</label>
            <input id="mm-name" v-model="linkName" class="input" placeholder="From the file name if empty" />
          </div>
          <div class="field">
            <label>Goes into</label>
            <div class="choices">
              <button class="choice" :class="{ on: linkTarget === 'content' }" @click="linkTarget = 'content'"><Box /> BONELAB Mods folder</button>
              <button class="choice" :class="{ on: linkTarget === 'code' }" @click="linkTarget = 'code'"><Code2 /> Game folder (MelonLoader)</button>
            </div>
          </div>
        </template>
        <p v-if="error" class="error-text selectable">{{ error }}</p>
        <div class="dialog-footer">
          <button class="btn" @click="emit('close')">Cancel</button>
          <button class="btn btn-primary" :disabled="!link.trim()" @click="resolve(link)">Continue</button>
        </div>
      </template>
    </template>
  </AppDialog>
</template>
