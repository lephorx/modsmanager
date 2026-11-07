<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { File, Folder, Link2, Loader2, Trash2 } from "lucide-vue-next";
import AppDialog from "./AppDialog.vue";
import { api, type ModView, type Unmanaged } from "@/lib/api";
import { refresh, store } from "@/lib/store";
import { errorText, toast } from "@/lib/toast";

/**
 * Files in the mod folders that no mod on the list put there: say which mod they are (so it
 * isn't downloaded again), keep them, or delete them. With `forMod`, it's "which of these are
 * that mod's files".
 */
const props = defineProps<{ game: number; forMod?: ModView }>();
const emit = defineEmits<{ close: [] }>();
const items = ref<Unmanaged[] | null>(null);
const picked = ref<Set<string>>(new Set());
/** which list mod each item is ("" = not on the list) */
const assign = ref<Record<string, string>>({});
const exact = ref(true);
const busy = ref(false);

/** Mods on the list this computer doesn't have (yet), which files could belong to. */
const candidates = computed(() => (store.overview?.mods ?? []).filter((m) => m.state !== "installed"));

const norm = (s: string) => s.toLowerCase().replace(/\.(dll|zip)$/, "").replace(/[^a-z0-9]/g, "");
/** The list mod a folder or file obviously is: same name, or one name inside the other. */
function suggest(item: Unmanaged): ModView | undefined {
  const base = norm(item.name.split("/").pop() ?? item.name);
  if (base.length < 3) return undefined;
  const scored = candidates.value
    .map((m) => {
      const n = norm(m.name);
      const score = n === base ? 3 : n.length >= 4 && (base.includes(n) || n.includes(base)) ? 2 : 0;
      return { m, score };
    })
    .filter((x) => x.score > 0)
    .sort((a, b) => b.score - a.score);
  // only when it's clear which one
  if (!scored.length || (scored[1] && scored[1].score === scored[0]!.score)) return undefined;
  return scored[0]!.m;
}

async function load() {
  items.value = await api.unmanaged(props.game).catch((e) => (toast.error(errorText(e)), []));
  const next: Record<string, string> = {};
  for (const i of items.value) {
    const s = props.forMod ? undefined : suggest(i);
    next[i.key] = assign.value[i.key] ?? (s ? String(s.id) : "");
  }
  assign.value = next;
  // for one mod: tick what looks like it; otherwise everything, for keep/delete
  picked.value = new Set(props.forMod ? items.value.filter((i) => suggestFor(i, props.forMod!)).map((i) => i.key) : items.value.map((i) => i.key));
}
function suggestFor(i: Unmanaged, m: ModView) {
  const base = norm(i.name.split("/").pop() ?? i.name);
  const n = norm(m.name);
  return base.length >= 3 && (base === n || (n.length >= 4 && (base.includes(n) || n.includes(base))));
}
onMounted(load);

const all = computed({
  get: () => !!items.value?.length && picked.value.size === items.value.length,
  set: (v) => (picked.value = new Set(v ? items.value?.map((i) => i.key) : [])),
});

function toggle(key: string) {
  const next = new Set(picked.value);
  next.has(key) ? next.delete(key) : next.add(key);
  picked.value = next;
}

/** item keys by the mod they're linked to */
const links = computed(() => {
  const by = new Map<number, string[]>();
  if (props.forMod) {
    if (picked.value.size) by.set(props.forMod.id, [...picked.value]);
    return by;
  }
  for (const [key, id] of Object.entries(assign.value)) {
    if (!id || !items.value?.some((i) => i.key === key)) continue;
    by.set(Number(id), [...(by.get(Number(id)) ?? []), key]);
  }
  return by;
});
const linkCount = computed(() => [...links.value.values()].reduce((n, k) => n + k.length, 0));
const modName = (id: number) => store.overview?.mods.find((m) => m.id === id)?.name ?? "the mod";

async function link() {
  busy.value = true;
  try {
    for (const [id, keys] of links.value) await api.adopt(props.game, id, keys, exact.value);
    const names = [...links.value.keys()].map(modName);
    toast.success(
      exact.value
        ? `${names.join(", ")} ${names.length === 1 ? "counts" : "count"} as installed now`
        : `Linked; the next sync replaces ${names.length === 1 ? "it" : "them"} with the version on the list`,
    );
    await refresh();
    if (props.forMod) return emit("close");
    await load();
    if (!items.value?.length) emit("close");
  } catch (e) {
    toast.error(errorText(e));
  } finally {
    busy.value = false;
  }
}

async function run(kind: "delete" | "keep") {
  // linked items aren't deleted or kept: they belong to a mod
  const keys = [...picked.value].filter((k) => !assign.value[k]);
  if (!keys.length) return;
  busy.value = true;
  try {
    if (kind === "delete") await api.deleteUnmanaged(props.game, keys);
    else await api.ignoreUnmanaged(props.game, keys);
    toast.success(kind === "delete" ? `Deleted ${keys.length}` : `Keeping ${keys.length}; they won't be asked about again`);
    await load();
    if (!items.value?.length) emit("close");
  } catch (e) {
    toast.error(errorText(e));
  } finally {
    busy.value = false;
  }
}

const unassignedPicked = computed(() => [...picked.value].filter((k) => !assign.value[k]).length);
</script>

<template>
  <AppDialog :title="forMod ? `Already have ${forMod.name}?` : 'Not on the list'" :width="680" @close="emit('close')">
    <p v-if="forMod" class="hint">
      Tick the folders or files in this game's mod folders that are {{ forMod.name }}. It then counts as installed instead of downloading again.
    </p>
    <p v-else class="hint">
      These are in this game's mod folders, but no mod on the list put them there (installed by hand or before). If one is a mod on the list, say
      which: it then counts as installed instead of downloading again.
    </p>
    <div v-if="!items" class="row hint" style="gap: 8px"><Loader2 class="spin" /> Looking…</div>
    <template v-else-if="items.length">
      <label class="row" style="gap: 8px"><input v-model="all" type="checkbox" /> Select all ({{ items.length }})</label>
      <div class="inset plan" style="max-height: 340px; overflow: auto">
        <div v-for="i in items" :key="i.key" class="plan-row unmanaged-row" :title="i.path">
          <input type="checkbox" :checked="picked.has(i.key)" :aria-label="`Select ${i.name}`" @change="toggle(i.key)" />
          <Folder v-if="i.isDir" :size="16" class="muted" /><File v-else :size="16" class="muted" />
          <span class="ellipsis" style="flex: 1; min-width: 0">{{ i.name }}</span>
          <span class="hint" style="flex-shrink: 0">{{ i.key.startsWith("game/") ? "Game folder" : "Mods folder" }}</span>
          <select v-if="!forMod" v-model="assign[i.key]" class="select assign-select" :aria-label="`Which mod is ${i.name}`" :class="{ set: assign[i.key] }">
            <option value="">Not on the list</option>
            <option v-for="m in candidates" :key="m.id" :value="String(m.id)">This is {{ m.name }}</option>
          </select>
        </div>
      </div>
      <label v-if="linkCount" class="row" style="gap: 8px; align-items: flex-start">
        <input v-model="exact" type="checkbox" style="margin-top: 3px" />
        <span class="stack" style="gap: 2px">
          <span>They're exactly the versions on the list</span>
          <span class="hint">Untick if you're not sure: they're linked anyway, and the next sync swaps them for the list's version.</span>
        </span>
      </label>
    </template>
    <p v-else class="hint">Nothing here: your folders only contain what's on the list.</p>
    <div class="dialog-footer">
      <template v-if="!forMod">
        <button class="btn" :disabled="busy || !unassignedPicked" @click="run('keep')">Keep selected</button>
        <button class="btn btn-danger" :disabled="busy || !unassignedPicked" @click="run('delete')"><Trash2 /> Delete selected</button>
      </template>
      <button v-else class="btn" @click="emit('close')">Cancel</button>
      <button v-if="linkCount || forMod" class="btn btn-primary" :disabled="busy || !linkCount" @click="link">
        <Loader2 v-if="busy" class="spin" /><Link2 v-else />
        {{ forMod ? `Link to ${forMod.name}` : `Link ${linkCount} to the list` }}
      </button>
    </div>
  </AppDialog>
</template>
