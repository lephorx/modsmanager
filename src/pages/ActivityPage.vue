<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { listen } from "@tauri-apps/api/event";
import { ArrowUpCircle, History, Import, Loader2, Minus, Plus } from "lucide-vue-next";
import { api, type ModEvent } from "@/lib/api";
import { ago } from "@/lib/format";
import { errorText } from "@/lib/toast";

const events = ref<ModEvent[] | null>(null);
const error = ref("");

async function load() {
  try {
    events.value = await api.events();
    error.value = "";
  } catch (e) {
    error.value = errorText(e);
  }
}

let unlisten: (() => void) | undefined;
onMounted(async () => {
  await load();
  unlisten = await listen("remote-changed", load);
});
onUnmounted(() => unlisten?.());

const ICON = { add: Plus, remove: Minus, update: ArrowUpCircle, import: Import } as const;
const VERB: Record<string, string> = { add: "added", remove: "removed", update: "updated", import: "imported" };

/** Grouped by day, newest first. */
const days = computed(() => {
  const groups: { day: string; items: ModEvent[] }[] = [];
  for (const e of events.value ?? []) {
    const d = new Date(e.at);
    const today = new Date();
    const yesterday = new Date(Date.now() - 86_400_000);
    const day =
      d.toDateString() === today.toDateString()
        ? "Today"
        : d.toDateString() === yesterday.toDateString()
          ? "Yesterday"
          : d.toLocaleDateString(undefined, { weekday: "long", day: "numeric", month: "long" });
    const last = groups[groups.length - 1];
    if (last?.day === day) last.items.push(e);
    else groups.push({ day, items: [e] });
  }
  return groups;
});
</script>

<template>
  <div class="page enter">
    <div class="page-header">
      <div>
        <h1 class="page-title">Activity</h1>
        <p class="page-sub">Who changed the list, and when</p>
      </div>
    </div>
    <p v-if="error" class="error-text selectable">{{ error }}</p>
    <div v-if="!events && !error" class="row hint" style="gap: 8px"><Loader2 class="spin" /> Loading…</div>
    <div v-else-if="events && !events.length" class="card empty">
      <History />
      <h3>Nothing yet</h3>
      <p>Adding, updating and removing mods shows up here.</p>
    </div>
    <div v-for="g in days" :key="g.day" class="stack" style="gap: 10px; margin-bottom: 24px">
      <h2 class="section-title">{{ g.day }}</h2>
      <div class="card mod-list">
        <div v-for="e in g.items" :key="e.id" class="mod-row event-row">
          <span class="event-icon" :data-action="e.action"><component :is="ICON[e.action as keyof typeof ICON] ?? History" :size="16" /></span>
          <span class="ellipsis" style="flex: 1">
            <strong>{{ e.actor }}</strong> {{ VERB[e.action] ?? e.action }} <strong>{{ e.modName }}</strong>
            <span v-if="e.detail" class="hint num"> · {{ e.detail.replace(" -> ", " → ") }}</span>
          </span>
          <span class="hint" :title="new Date(e.at).toLocaleString()">{{ ago(e.at) }}</span>
        </div>
      </div>
    </div>
  </div>
</template>
