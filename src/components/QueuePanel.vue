<script setup lang="ts">
import { computed, ref } from "vue";
import { ArrowUpToLine, Ban, ChevronDown, ChevronUp, CircleCheck, CircleStop, Clock, Download, ExternalLink, GripVertical, Loader2, OctagonX, RotateCw, Trash2, X } from "lucide-vue-next";
import { api, isActive, type Job } from "@/lib/api";
import { store } from "@/lib/store";
import { bytes } from "@/lib/format";
import { errorText, toast } from "@/lib/toast";

/** The install queue: what's running, waiting for a Nexus click, queued, and done. */
const groups = computed(() => {
  const q = store.queue;
  const running = q.filter((j) => j.status === "downloading" || j.status === "installing" || j.status === "removing");
  return [
    { title: "Running", jobs: running },
    { title: "Waiting for a click on Nexus", jobs: q.filter((j) => j.status === "waiting") },
    { title: "Queued", jobs: q.filter((j) => j.status === "queued") },
    { title: "Finished", jobs: q.filter((j) => j.status === "done" || j.status === "failed" || j.status === "cancelled").reverse() },
  ].filter((g) => g.jobs.length);
});
const finished = computed(() => store.queue.some((j) => j.status === "done" || j.status === "failed" || j.status === "cancelled"));
/** something that Cancel all would stop */
const stoppable = computed(() => store.queue.some((j) => j.status === "queued" || j.status === "waiting" || j.status === "downloading"));
const multiGame = computed(() => new Set(store.queue.map((j) => j.game)).size > 1);

const ACTION: Record<string, string> = { install: "Install", update: "Update", repair: "Repair", remove: "Remove" };

function detail(j: Job) {
  if (j.status === "downloading") return j.total ? `${bytes(j.done)} of ${bytes(j.total)}` : bytes(j.done) || "Starting…";
  if (j.status === "installing") return "Installing…";
  if (j.status === "removing") return "Removing…";
  if (j.status === "waiting") return "Click Slow download on its Nexus page";
  if (j.status === "failed") return j.error ?? "Failed";
  if (j.status === "cancelled") return "Cancelled; Sync now gets it again";
  if (j.status === "done") return j.action === "remove" ? "Removed" : "Installed";
  return j.total ? bytes(j.total) : "";
}

function percent(j: Job) {
  if (j.status === "downloading") return j.total ? Math.round((j.done / j.total) * 100) : 2;
  if (j.status === "installing") return 96;
  if (j.status === "removing") return 50;
  return 0;
}

// ── moving queued jobs: the queue's order is the download order ──
const queuedIds = computed(() => store.queue.filter((j) => j.status === "queued").map((j) => j.id));

/** Shows the new order right away; the backend's answer confirms it. */
function reorder(order: number[]) {
  const pos = new Map(order.map((id, i) => [id, i]));
  const queued = store.queue.filter((j) => j.status === "queued").sort((a, b) => (pos.get(a.id) ?? 1e9) - (pos.get(b.id) ?? 1e9));
  let k = 0;
  store.queue = store.queue.map((j) => (j.status === "queued" ? queued[k++]! : j));
  void run(() => api.queueReorder(order));
}

function move(id: number, to: number) {
  const ids = queuedIds.value.filter((x) => x !== id);
  ids.splice(Math.max(0, Math.min(to, ids.length)), 0, id);
  reorder(ids);
}
const shift = (id: number, by: number) => move(id, queuedIds.value.indexOf(id) + by);

const dragging = ref<number | null>(null);
const over = ref<{ id: number; after: boolean } | null>(null);

function dragOver(e: DragEvent, id: number) {
  if (dragging.value == null || dragging.value === id) return (over.value = null);
  const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
  over.value = { id, after: e.clientY > r.top + r.height / 2 };
}

function drop() {
  const id = dragging.value;
  const target = over.value;
  dragging.value = null;
  over.value = null;
  if (id == null || !target) return;
  const ids = queuedIds.value.filter((x) => x !== id);
  ids.splice(ids.indexOf(target.id) + (target.after ? 1 : 0), 0, id);
  reorder(ids);
}

async function run(f: () => Promise<unknown>) {
  try {
    await f();
  } catch (e) {
    toast.error(errorText(e));
  }
}
</script>

<template>
  <Teleport to="body">
    <div class="queue-backdrop" @mousedown.self="store.queueOpen = false">
      <div class="account-menu queue-panel" role="dialog" aria-label="Downloads">
        <div class="queue-head">
          <strong>Downloads</strong>
          <span class="hint">{{ store.queue.filter(isActive).length ? `${store.queue.filter(isActive).length} to go` : "Nothing running" }}</span>
          <span class="spacer" />
          <button v-if="stoppable" class="btn btn-sm btn-ghost" @click="run(api.queueCancelAll)"><CircleStop /> Cancel all</button>
          <button v-if="finished" class="btn btn-sm btn-ghost" @click="run(api.queueClear)"><Trash2 /> Clear finished</button>
          <button class="btn btn-sm btn-ghost btn-icon" title="Close" aria-label="Close" @click="store.queueOpen = false"><X /></button>
        </div>
        <div v-if="!groups.length" class="empty" style="padding: 36px 16px">
          <Download />
          <span>Nothing queued. Sync now puts what this computer is missing here.</span>
        </div>
        <div v-else class="queue-list">
          <template v-for="g in groups" :key="g.title">
            <span class="queue-group">{{ g.title }}<span v-if="g.title === 'Queued' && g.jobs.length > 1" class="queue-group-hint"> · drag or use the arrows to change the order</span></span>
            <div
              v-for="j in g.jobs"
              :key="j.id"
              class="queue-job"
              :class="{
                dragging: dragging === j.id,
                'drop-before': over?.id === j.id && !over.after,
                'drop-after': over?.id === j.id && over.after,
              }"
              :data-status="j.status"
              :draggable="j.status === 'queued'"
              @dragstart="j.status === 'queued' && (dragging = j.id)"
              @dragover.prevent="j.status === 'queued' && dragOver($event, j.id)"
              @drop.prevent="drop()"
              @dragend="(dragging = null), (over = null)"
            >
              <span v-if="j.status === 'queued' && queuedIds.length > 1" class="queue-grip" title="Drag to change the order"><GripVertical /></span>
              <span class="queue-icon">
                <Loader2 v-if="j.status === 'downloading' || j.status === 'installing' || j.status === 'removing'" class="spin" />
                <CircleCheck v-else-if="j.status === 'done'" />
                <OctagonX v-else-if="j.status === 'failed'" />
                <Ban v-else-if="j.status === 'cancelled'" />
                <ExternalLink v-else-if="j.status === 'waiting'" />
                <Clock v-else />
              </span>
              <span class="stack" style="gap: 3px; flex: 1; min-width: 0">
                <span class="row" style="gap: 6px; min-width: 0">
                  <strong class="ellipsis">{{ j.name }}</strong>
                  <span class="hint" style="flex-shrink: 0">{{ ACTION[j.action] }}<template v-if="multiGame"> · {{ j.gameName }}</template></span>
                </span>
                <span class="hint ellipsis selectable" :title="j.error ?? undefined">{{ detail(j) }}</span>
                <span v-if="j.status === 'downloading' || j.status === 'installing' || j.status === 'removing'" class="progress queue-progress">
                  <span :style="{ width: `${Math.max(percent(j), 2)}%` }" />
                </span>
              </span>
              <span v-if="j.status === 'queued' && queuedIds.length > 1" class="queue-move">
                <button
                  v-if="queuedIds[0] !== j.id"
                  class="btn btn-sm btn-ghost btn-icon"
                  title="Download next"
                  :aria-label="`Download ${j.name} next`"
                  @click="move(j.id, 0)"
                >
                  <ArrowUpToLine />
                </button>
                <button class="btn btn-sm btn-ghost btn-icon" title="Move up" :aria-label="`Move ${j.name} up`" :disabled="queuedIds[0] === j.id" @click="shift(j.id, -1)">
                  <ChevronUp />
                </button>
                <button
                  class="btn btn-sm btn-ghost btn-icon"
                  title="Move down"
                  :aria-label="`Move ${j.name} down`"
                  :disabled="queuedIds[queuedIds.length - 1] === j.id"
                  @click="shift(j.id, 1)"
                >
                  <ChevronDown />
                </button>
              </span>
              <button v-if="j.status === 'waiting' && j.manualUrl" class="btn btn-sm" @click="run(() => api.openUrl(j.manualUrl!))"><ExternalLink /> Open</button>
              <button v-if="j.status === 'failed'" class="btn btn-sm" @click="run(() => api.queueRetry(j.id))"><RotateCw /> Retry</button>
              <button v-if="j.status === 'cancelled'" class="btn btn-sm" @click="run(() => api.queueRetry(j.id))"><RotateCw /> Start again</button>
              <button
                v-if="j.status === 'queued' || j.status === 'waiting' || j.status === 'downloading'"
                class="btn btn-sm btn-ghost btn-icon"
                :title="j.status === 'downloading' ? 'Cancel download' : 'Take off the queue'"
                :aria-label="j.status === 'downloading' ? `Cancel download of ${j.name}` : `Take ${j.name} off the queue`"
                @click="run(() => api.queueCancel(j.id))"
              >
                <X />
              </button>
            </div>
          </template>
        </div>
      </div>
    </div>
  </Teleport>
</template>
