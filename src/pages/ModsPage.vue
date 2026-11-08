<script setup lang="ts">
import { computed, ref } from "vue";
import {
  ArrowUpCircle,
  CircleCheck,
  Code2,
  Download,
  ExternalLink,
  FolderOpen,
  Layers,
  Loader2,
  Package,
  Plus,
  RefreshCw,
  Search,
  Trash2,
  TriangleAlert,
  Box,
  Gamepad2,
  FolderCog,
  ListChecks,
  UserPlus,
  Link2,
  RotateCcw,
} from "lucide-vue-next";
import { api, isActive, LOADER_LABEL, type Action, type Job, type ModView } from "@/lib/api";
import { checkUpdates, currentGame, gameJobs, hasPaths, nextFromNexus, nexusPending, refresh, runSync, stopNexus, store } from "@/lib/store";
import { gameDialog } from "@/lib/dialogs";
import { ago, hideImg, bytes, count, SOURCE_LABEL } from "@/lib/format";
import { errorText, toast } from "@/lib/toast";
import { openSettings } from "@/lib/settingsDialog";
import AddModDialog from "@/components/AddModDialog.vue";
import AppDialog from "@/components/AppDialog.vue";
import RowMenu, { type RowMenuItem } from "@/components/RowMenu.vue";
import UnmanagedDialog from "@/components/UnmanagedDialog.vue";

const tab = ref<"all" | "content" | "code">("all");
const query = ref("");
const adding = ref(false);
const reviewing = ref(false);
/** "I already have this": link files in the folders to this mod */
const adopting = ref<ModView | null>(null);
const confirmRemove = ref<ModView | null>(null);
const busy = ref<Record<number, boolean>>({});

const o = computed(() => store.overview);
const game = computed(() => store.games?.find((g) => g.id === currentGame()) ?? null);
const loader = computed(() => (o.value ? LOADER_LABEL[o.value.game.loader] : ""));
const LOADER_HELP: Record<string, { text: string; url: string }> = {
  melonloader: { text: "Install it with the MelonLoader installer.", url: "https://github.com/LavaGang/MelonLoader.Installer/releases/latest" },
  bepinex: { text: "Add BepInExPack from Thunderstore to the list (Add mod), or install it by hand.", url: "https://thunderstore.io/package/bbepis/BepInExPack/" },
};
const mods = computed(() => o.value?.mods ?? []);
const shown = computed(() => {
  const q = query.value.trim().toLowerCase();
  return mods.value.filter(
    (m) =>
      (tab.value === "all" || m.target === tab.value) &&
      (!q || m.name.toLowerCase().includes(q) || m.author.toLowerCase().includes(q)),
  );
});
const counts = computed(() => ({
  all: mods.value.length,
  content: mods.value.filter((m) => m.target === "content").length,
  code: mods.value.filter((m) => m.target === "code").length,
}));
const plan = computed(() => o.value?.plan ?? []);
const manual = computed(() => nexusPending());
/** what Sync now can do on its own */
const automatic = computed(() => plan.value.filter((p) => !p.manualUrl));
const planSize = computed(() => plan.value.reduce((n, p) => n + (p.action === "remove" ? 0 : p.size), 0));
const updateCount = computed(() => Object.keys(store.updates).length);

const ACTION: Record<Action, { label: string; cls: string }> = {
  install: { label: "Install", cls: "a-install" },
  update: { label: "Update", cls: "a-update" },
  repair: { label: "Repair", cls: "a-update" },
  remove: { label: "Remove", cls: "a-remove" },
};

const STATE: Record<ModView["state"], { label: string; state: string }> = {
  installed: { label: "Installed", state: "running" },
  missing: { label: "Not installed", state: "idle" },
  outdated: { label: "Other version", state: "preparing" },
  broken: { label: "Files missing", state: "error" },
};

/** This game's jobs in the current run of the queue (from the oldest one still going). */
const run = computed(() => {
  const jobs = gameJobs();
  const active = jobs.filter(isActive);
  if (!active.length) return [];
  const from = Math.min(...active.map((j) => j.id));
  return jobs.filter((j) => j.id >= from && j.status !== "waiting");
});
const installing = computed(() => run.value.some(isActive));

function share(j: Job) {
  if (j.status === "done" || j.status === "failed") return 1;
  if (j.status === "downloading") return j.total ? (j.done / j.total) * 0.9 : 0.05;
  if (j.status === "installing") return 0.95;
  if (j.status === "removing") return 0.5;
  return 0;
}
const percent = computed(() => (run.value.length ? Math.round((run.value.reduce((n, j) => n + share(j), 0) / run.value.length) * 100) : 0));

const phaseText = computed(() => {
  const going = run.value.filter((j) => j.status === "downloading" || j.status === "installing" || j.status === "removing");
  const left = run.value.filter(isActive).length;
  const now = going.map((j) => (j.status === "downloading" ? `${j.name}${j.total ? ` (${Math.round((j.done / j.total) * 100)}%)` : ""}` : `${j.name} (${j.status})`));
  return `${left} left${now.length ? ` · ${now.join(", ")}` : ""}`;
});

async function updateMod(m: ModView) {
  busy.value[m.id] = true;
  try {
    const v = await api.update(m.id);
    if (v) {
      toast.success(`${m.name} now pinned to ${v} for everyone`);
      delete store.updates[m.id];
      await refresh();
      if (store.overview?.plan.length) void runSync(true);
    } else toast.info(`${m.name} is already on its newest file`);
  } catch (e) {
    toast.error(errorText(e));
  } finally {
    busy.value[m.id] = false;
  }
}

async function updateAll() {
  for (const id of Object.keys(store.updates)) {
    const m = mods.value.find((x) => x.id === Number(id));
    if (m) await updateMod(m);
  }
}

async function removeMod(m: ModView) {
  confirmRemove.value = null;
  try {
    await api.remove(m.id);
    toast.success(`${m.name} is off the list`);
    await refresh();
    void runSync(true);
  } catch (e) {
    toast.error(errorText(e));
  }
}

async function reinstall(m: ModView) {
  try {
    await api.reinstall(m.gameId, m.id);
    toast.info(`Reinstalling ${m.name}…`);
    await refresh();
    await runSync(true);
  } catch (e) {
    toast.error(errorText(e));
  }
}

function menu(m: ModView): RowMenuItem[] {
  const items: RowMenuItem[] = [];
  if (m.pageUrl) items.push({ label: m.source === "link" ? "Open link" : `Open on ${SOURCE_LABEL[m.source]}`, icon: ExternalLink, action: () => void api.openUrl(m.pageUrl) });
  items.push({
    label: m.source === "link" ? "Re-download and re-pin" : "Update to newest",
    icon: RefreshCw,
    action: () => void updateMod(m),
  });
  items.push({
    label: "Open folder",
    icon: FolderOpen,
    action: () => void api.openFolder(m.gameId, m.target === "code" && o.value?.game.loader !== "none" ? "game" : "content").catch((e) => toast.error(errorText(e))),
  });
  if (m.state !== "missing") items.push({ label: "Reinstall", icon: RotateCcw, action: () => void reinstall(m) });
  if (m.state !== "installed") items.push({ label: "I already have this…", icon: Link2, action: () => (adopting.value = m) });
  items.push({ label: "Remove for everyone", icon: Trash2, danger: true, action: () => (confirmRemove.value = m) });
  return items;
}

function added(m: ModView) {
  return m.updatedAt !== m.addedAt && m.updatedBy
    ? `Updated by ${m.updatedBy} ${ago(m.updatedAt)}`
    : `Added by ${m.addedBy} ${ago(m.addedAt)}`;
}
</script>

<template>
  <div class="page enter">
    <div class="page-header">
      <div>
        <h1 class="page-title">{{ game?.name ?? "Mods" }}</h1>
        <p class="page-sub">
          <template v-if="game">{{ count(mods.length, "mod") }} on the shared list<template v-if="o && !plan.length"> · this computer matches it</template></template>
          <template v-else>Mods you share, kept identical on every computer</template>
        </p>
      </div>
      <div class="row" style="gap: 8px">
        <button v-if="game" class="btn" :disabled="store.checking || !mods.length" @click="checkUpdates()">
          <RefreshCw :class="{ spin: store.checking }" /> Check for updates
        </button>
        <button v-if="game" class="btn btn-primary" @click="adding = true"><Plus /> Add mod</button>
      </div>
    </div>

    <div v-if="store.error" class="card notice notice-error">
      <TriangleAlert />
      <div class="stack" style="gap: 2px; flex: 1">
        <strong>Can't load the shared list</strong>
        <span class="hint selectable">{{ store.error }}</span>
      </div>
      <button class="btn" @click="openSettings('shared')">Settings</button>
      <button class="btn" @click="refresh()">Retry</button>
    </div>

    <div v-if="store.games && !store.games.length" class="card empty">
      <Gamepad2 />
      <h3>No games yet</h3>
      <p>Add the game you play together. Everyone on the database sees it, with its own mod list.</p>
      <div class="row" style="gap: 8px">
        <button class="btn btn-primary" @click="gameDialog.add = true"><Plus /> Add a game</button>
      </div>
    </div>

    <section v-else-if="game && !hasPaths(game.id)" class="card sync-card">
      <div class="row" style="gap: 14px; align-items: flex-start">
        <span class="sync-icon"><FolderCog /></span>
        <div class="stack" style="gap: 2px; flex: 1; min-width: 0">
          <h2 class="card-title">Where is {{ game.name }} on this computer?</h2>
          <span class="hint">Choose its folders once, then everything on the list installs with one click.</span>
        </div>
        <button class="btn btn-primary btn-lg" @click="gameDialog.paths = game"><FolderCog /> Set up folders</button>
      </div>
    </section>

    <!-- what this computer has to do to match the list -->
    <section v-else-if="installing || plan.length" class="card sync-card">
      <div class="row" style="gap: 14px; align-items: flex-start">
        <span class="sync-icon"><Download v-if="!installing" /><Loader2 v-else class="spin" /></span>
        <div class="stack" style="gap: 2px; flex: 1; min-width: 0">
          <h2 class="card-title">{{ installing ? `Installing… ${percent}%` : `${count(plan.length, "change")} to match the list` }}</h2>
          <span class="hint">
            <template v-if="installing">{{ phaseText }}</template>
            <template v-else>Someone changed the list, or something here doesn't match it.<template v-if="planSize"> About {{ bytes(planSize) }} to download.</template></template>
          </span>
        </div>
        <template v-if="installing">
          <button class="btn btn-lg btn-ghost" @click="api.queueCancelAll().catch((e) => toast.error(errorText(e)))">Cancel</button>
          <button class="btn btn-lg" @click="store.queueOpen = true"><ListChecks /> Downloads</button>
        </template>
        <button v-else-if="automatic.length" class="btn btn-play btn-lg" @click="runSync()"><Download /> Sync now</button>
        <button v-else-if="manual.length && !store.nexusAssist" class="btn btn-primary btn-lg" @click="nextFromNexus()"><ExternalLink /> Get from Nexus</button>
      </div>
      <!-- Free Nexus accounts use the saved browser session. -->
      <div v-if="manual.length" class="inset nexus-strip">
        <template v-if="store.nexusAssist">
          <Loader2 class="spin" />
          <span class="stack" style="gap: 2px; flex: 1; min-width: 0">
            <strong class="ellipsis">Waiting for {{ manual[0]!.name }} from Nexus</strong>
            <span class="hint">Downloads start automatically. Log in if the Nexus window appears; your session is remembered.</span>
          </span>
          <button class="btn btn-sm" @click="nextFromNexus(true)">Show Nexus</button>
          <button class="btn btn-sm btn-ghost" @click="stopNexus()">Stop</button>
        </template>
        <template v-else>
          <ExternalLink />
          <span class="stack" style="gap: 2px; flex: 1; min-width: 0">
            <strong>{{ count(manual.length, "mod") }} from Nexus Mods {{ manual.length === 1 ? "needs" : "need" }} a browser session</strong>
            <span class="hint">{{ store.settings?.nexusLoggedIn ? "Mods Manager selects Slow download automatically in the background." : "Log in to Nexus Mods in Settings first; then Mods Manager selects Slow download automatically." }}</span>
          </span>
          <button v-if="automatic.length" class="btn" @click="nextFromNexus()">Get {{ manual.length }} from Nexus</button>
        </template>
      </div>
      <div v-if="installing" class="progress" style="margin-top: 14px"><div :style="{ width: `${Math.max(percent, 3)}%` }" /></div>
      <div v-else class="inset plan">
        <div v-for="p in plan" :key="p.key" class="plan-row">
          <span class="plan-action" :class="ACTION[p.action].cls">{{ ACTION[p.action].label }}</span>
          <span class="ellipsis" style="font-weight: 600">{{ p.name }}</span>
          <span v-if="p.manualUrl" class="chip" title="Downloads through the saved Nexus session">Nexus · click</span>
          <span class="spacer" />
          <span class="hint num">
            <template v-if="p.action === 'update'">{{ p.from }} → {{ p.to }}</template>
            <template v-else-if="p.action === 'remove'">{{ p.from }}</template>
            <template v-else>{{ p.to }}<template v-if="p.size"> · {{ bytes(p.size) }}</template></template>
          </span>
        </div>
      </div>
    </section>
    <section v-else-if="o && mods.length" class="card sync-card synced">
      <div class="row" style="gap: 14px">
        <span class="sync-icon ok"><CircleCheck /></span>
        <div class="stack" style="gap: 2px; flex: 1">
          <h2 class="card-title">In sync</h2>
          <span class="hint">Every mod on the list is installed here, in exactly the pinned version.</span>
        </div>
      </div>
    </section>

    <div v-if="o && game && hasPaths(game.id) && o.needsLoader && !o.loaderOk && !plan.some((p) => /bepinexpack/i.test(p.key))" class="card notice">
      <TriangleAlert />
      <div class="stack" style="gap: 2px; flex: 1">
        <strong>{{ loader }} isn't installed</strong>
        <span class="hint">Code mods on the list need {{ loader }} in the {{ o.game.name }} folder. {{ LOADER_HELP[o.game.loader]?.text }} Or check the game folder (Edit game).</span>
      </div>
      <button v-if="LOADER_HELP[o.game.loader]" class="btn" @click="api.openUrl(LOADER_HELP[o.game.loader]!.url)">Get {{ loader }}</button>
    </div>

    <div v-if="updateCount" class="card notice notice-tint">
      <ArrowUpCircle />
      <div class="stack" style="gap: 2px; flex: 1">
        <strong>{{ count(updateCount, "mod has", "mods have") }} a newer version</strong>
        <span class="hint">Updating pins the new file for everyone; the others get it on their next sync.</span>
      </div>
      <button class="btn btn-tint" @click="updateAll"><ArrowUpCircle /> Update all</button>
    </div>

    <div v-if="o?.unmanaged" class="card notice">
      <Package />
      <div class="stack" style="gap: 2px; flex: 1">
        <strong>{{ count(o.unmanaged, "item") }} in your mod folders {{ o.unmanaged === 1 ? "isn't" : "aren't" }} on the list</strong>
        <span class="hint">If {{ o.unmanaged === 1 ? "it's" : "some are" }} a mod on the list, link {{ o.unmanaged === 1 ? "it" : "them" }} so nothing downloads again. Otherwise keep or delete.</span>
      </div>
      <button class="btn" @click="reviewing = true">Review</button>
    </div>

    <template v-if="game">
    <div class="list-head">
      <div v-slider class="tabs" role="tablist">
        <button :class="{ active: tab === 'all' }" role="tab" @click="tab = 'all'"><Layers /> All <span class="tab-count">{{ counts.all }}</span></button>
        <button :class="{ active: tab === 'content' }" role="tab" @click="tab = 'content'"><Box /> Content <span class="tab-count">{{ counts.content }}</span></button>
        <button :class="{ active: tab === 'code' }" role="tab" @click="tab = 'code'"><Code2 /> Code <span class="tab-count">{{ counts.code }}</span></button>
      </div>
      <span class="spacer" />
      <label class="input-icon" style="width: 260px">
        <Search />
        <input v-model="query" class="input" placeholder="Search the list" aria-label="Search the list" />
      </label>
    </div>

    <div v-if="!o && store.loading" class="card mod-list">
      <div v-for="i in 4" :key="i" class="mod-row"><span class="mod-thumb skeleton-line" /><span class="skeleton-line" style="width: 40%; height: 14px" /></div>
    </div>
    <div v-else-if="o && !mods.length" class="card empty">
      <Layers />
      <h3>The list is empty</h3>
      <p>Add the first mod; everyone on the list gets it too.</p>
      <button class="btn btn-primary" @click="adding = true"><Plus /> Add mod</button>
    </div>
    <div v-else-if="o && !shown.length" class="card empty">
      <Search />
      <h3>Nothing matches</h3>
    </div>
    <div v-else-if="o" class="card mod-list">
      <div v-for="m in shown" :key="m.id" class="mod-row">
        <span class="mod-thumb">
          <Code2 v-if="m.target === 'code'" /><Box v-else />
          <img v-if="m.thumbUrl" :src="m.thumbUrl" alt="" loading="lazy" @error="hideImg" />
        </span>
        <div class="stack mod-main">
          <div class="row" style="gap: 8px; min-width: 0">
            <span class="mod-name ellipsis" :title="m.name">{{ m.name }}</span>
            <span class="chip">{{ SOURCE_LABEL[m.source] }}</span>
            <span class="chip num" :title="m.fileHash ? `Pinned file ${m.fileRef || ''} ${m.fileHash}` : undefined">{{ m.version }}</span>
            <span v-if="m.target === 'code'" class="chip"><Code2 /> Code</span>
          </div>
          <span class="hint ellipsis">
            <template v-if="m.author">by {{ m.author }} · </template>{{ added(m) }}<template v-if="m.fileSize"> · {{ bytes(m.fileSize) }}</template>
          </span>
        </div>
        <button v-if="store.updates[m.id]" class="btn btn-sm btn-tint" :disabled="busy[m.id]" :title="`Pin ${store.updates[m.id]!.version} for everyone`" @click="updateMod(m)">
          <Loader2 v-if="busy[m.id]" class="spin" /><ArrowUpCircle v-else /> {{ store.updates[m.id]!.version }}
        </button>
        <span class="badge" :data-state="STATE[m.state].state" :title="m.state === 'outdated' ? `You have ${m.localVersion}` : undefined">
          <span class="dot" />{{ STATE[m.state].label }}
        </span>
        <RowMenu :items="menu(m)" :label="`More for ${m.name}`" />
      </div>
    </div>

    </template>

    <AddModDialog v-if="adding" @close="adding = false" />
    <UnmanagedDialog v-if="reviewing && game" :game="game.id" @close="(reviewing = false), refresh()" />
    <UnmanagedDialog v-if="adopting && game" :game="game.id" :for-mod="adopting" @close="(adopting = null), refresh()" />
    <AppDialog v-if="confirmRemove" title="Remove for everyone?" :width="440" @close="confirmRemove = null">
      <p>
        <strong>{{ confirmRemove.name }}</strong> comes off the shared list. Its files are deleted on this computer now, and on everyone
        else's computer on their next sync.
      </p>
      <div class="dialog-footer">
        <button class="btn" @click="confirmRemove = null">Cancel</button>
        <button class="btn btn-danger" @click="removeMod(confirmRemove!)"><Trash2 /> Remove</button>
      </div>
    </AppDialog>
  </div>
</template>
