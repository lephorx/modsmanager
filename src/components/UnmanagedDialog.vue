<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { File, Folder, Loader2, Trash2 } from "lucide-vue-next";
import AppDialog from "./AppDialog.vue";
import { api, type Unmanaged } from "@/lib/api";
import { errorText, toast } from "@/lib/toast";

/** Files in the mod folders that no mod on the list put there: delete them or keep them. */
const emit = defineEmits<{ close: [] }>();
const items = ref<Unmanaged[] | null>(null);
const picked = ref<Set<string>>(new Set());
const busy = ref(false);

async function load() {
  items.value = await api.unmanaged().catch((e) => (toast.error(errorText(e)), []));
  picked.value = new Set(items.value.map((i) => i.key));
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

async function run(kind: "delete" | "keep") {
  const keys = [...picked.value];
  if (!keys.length) return;
  busy.value = true;
  try {
    if (kind === "delete") await api.deleteUnmanaged(keys);
    else await api.ignoreUnmanaged(keys);
    toast.success(kind === "delete" ? `Deleted ${keys.length}` : `Keeping ${keys.length}; they won't be asked about again`);
    await load();
    if (!items.value?.length) emit("close");
  } catch (e) {
    toast.error(errorText(e));
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <AppDialog title="Not on the list" :width="600" @close="emit('close')">
    <p class="hint">
      These are in your BONELAB mod folders, but no mod on the shared list put them there (for example mods from before, or installed by hand).
      Your friend doesn't have them.
    </p>
    <div v-if="!items" class="row hint" style="gap: 8px"><Loader2 class="spin" /> Looking…</div>
    <template v-else-if="items.length">
      <label class="row" style="gap: 8px"><input v-model="all" type="checkbox" /> Select all ({{ items.length }})</label>
      <div class="inset plan" style="max-height: 340px; overflow: auto">
        <label v-for="i in items" :key="i.key" class="plan-row" :title="i.path">
          <input type="checkbox" :checked="picked.has(i.key)" @change="toggle(i.key)" />
          <Folder v-if="i.isDir" :size="16" class="muted" /><File v-else :size="16" class="muted" />
          <span class="ellipsis">{{ i.name }}</span>
          <span class="spacer" />
          <span class="hint">{{ i.key.startsWith("game/") ? "Game folder" : "Mods folder" }}</span>
        </label>
      </div>
    </template>
    <p v-else class="hint">Nothing here: your folders only contain what's on the list.</p>
    <div class="dialog-footer">
      <button class="btn" :disabled="busy || !picked.size" @click="run('keep')">Keep selected</button>
      <button class="btn btn-danger" :disabled="busy || !picked.size" @click="run('delete')"><Trash2 /> Delete selected</button>
    </div>
  </AppDialog>
</template>
