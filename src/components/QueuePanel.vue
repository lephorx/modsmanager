<script setup lang="ts">
import { computed } from "vue";
import { CircleCheck, Clock, Download, ExternalLink, Loader2, OctagonX, RotateCw, Trash2, X } from "lucide-vue-next";
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
    { title: "Finished", jobs: q.filter((j) => j.status === "done" || j.status === "failed").reverse() },
  ].filter((g) => g.jobs.length);
});
const finished = computed(() => store.queue.some((j) => j.status === "done" || j.status === "failed"));
const multiGame = computed(() => new Set(store.queue.map((j) => j.game)).size > 1);

const ACTION: Record<string, string> = { install: "Install", update: "Update", repair: "Repair", remove: "Remove" };

function detail(j: Job) {
  if (j.status === "downloading") return j.total ? `${bytes(j.done)} of ${bytes(j.total)}` : bytes(j.done) || "Starting…";
  if (j.status === "installing") return "Installing…";
  if (j.status === "removing") return "Removing…";
  if (j.status === "waiting") return "Click Slow download on its Nexus page";
  if (j.status === "failed") return j.error ?? "Failed";
  if (j.status === "done") return j.action === "remove" ? "Removed" : "Installed";
  return j.total ? bytes(j.total) : "";
}

function percent(j: Job) {
  if (j.status === "downloading") return j.total ? Math.round((j.done / j.total) * 100) : 2;
  if (j.status === "installing") return 96;
  if (j.status === "removing") return 50;
  return 0;
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
          <button v-if="finished" class="btn btn-sm btn-ghost" @click="run(api.queueClear)"><Trash2 /> Clear finished</button>
          <button class="btn btn-sm btn-ghost btn-icon" title="Close" aria-label="Close" @click="store.queueOpen = false"><X /></button>
        </div>
        <div v-if="!groups.length" class="empty" style="padding: 36px 16px">
          <Download />
          <span>Nothing queued. Sync now puts what this computer is missing here.</span>
        </div>
        <div v-else class="queue-list">
          <template v-for="g in groups" :key="g.title">
            <span class="queue-group">{{ g.title }}</span>
            <div v-for="j in g.jobs" :key="j.id" class="queue-job" :data-status="j.status">
              <span class="queue-icon">
                <Loader2 v-if="j.status === 'downloading' || j.status === 'installing' || j.status === 'removing'" class="spin" />
                <CircleCheck v-else-if="j.status === 'done'" />
                <OctagonX v-else-if="j.status === 'failed'" />
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
              <button v-if="j.status === 'waiting' && j.manualUrl" class="btn btn-sm" @click="run(() => api.openUrl(j.manualUrl!))"><ExternalLink /> Open</button>
              <button v-if="j.status === 'failed'" class="btn btn-sm" @click="run(() => api.queueRetry(j.id))"><RotateCw /> Retry</button>
              <button
                v-if="j.status === 'queued' || j.status === 'waiting'"
                class="btn btn-sm btn-ghost btn-icon"
                title="Take off the queue"
                aria-label="Take off the queue"
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
