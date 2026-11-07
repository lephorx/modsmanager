<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { RouterLink, RouterView } from "vue-router";
import { Check, ChevronDown, Download, Gamepad2, History, Layers, Loader2, Pencil, Plus, RefreshCw, Settings as SettingsIcon, UserPlus } from "lucide-vue-next";
import markUrl from "./assets/mark.svg";
import markWhiteUrl from "./assets/mark-white.svg";
import Toast from "./components/AppToast.vue";
import SettingsDialog from "./components/SettingsDialog.vue";
import GameDialog from "./components/GameDialog.vue";
import InviteDialog from "./components/InviteDialog.vue";
import QueuePanel from "./components/QueuePanel.vue";
import { isActive } from "./lib/api";
import SetupPage from "./pages/SetupPage.vue";
import { settingsDialog, openSettings } from "./lib/settingsDialog";
import { gameDialog } from "./lib/dialogs";
import { ONLINE_MS, currentGame, loadSettings, refresh, selectGame, setupDone, startEvents, store } from "./lib/store";
import { ago } from "./lib/format";
import { checkForUpdate, installUpdate, startUpdateChecks, updater } from "./lib/updater";

const scrolled = ref(false);
const ready = ref(false);
const menu = ref<"" | "games" | "people">("");
const inviting = ref(false);

function closeMenus(e: MouseEvent) {
  if (!(e.target as HTMLElement).closest(".menu-wrap")) menu.value = "";
}

onMounted(async () => {
  window.addEventListener("mousedown", closeMenus);
  await loadSettings();
  ready.value = true;
  await startEvents();
  void startUpdateChecks();
  await refresh();
});
onUnmounted(() => window.removeEventListener("mousedown", closeMenus));

const game = computed(() => store.games?.find((g) => g.id === currentGame()) ?? null);

/** Everyone on the database: online or not, and where they stand in this game. */
const people = computed(() => {
  const o = store.overview;
  if (!o) return [];
  return o.members
    .map((m) => ({ ...m, me: m.name === o.me, online: m.name === o.me || Date.now() - new Date(m.lastSeen).getTime() < ONLINE_MS }))
    .sort((a, b) => Number(b.me) - Number(a.me) || Number(b.online) - Number(a.online) || a.name.localeCompare(b.name));
});
const others = computed(() => people.value.filter((p) => !p.me));
const queueBusy = computed(() => store.queue.filter(isActive).length);
const queueWaiting = computed(() => store.queue.filter((j) => j.status === "waiting").length);
const queueFailed = computed(() => store.queue.filter((j) => j.status === "failed").length);
const behind = computed(() => others.value.filter((p) => p.pending).length);

function stateText(p: (typeof people.value)[number]) {
  if (p.pending == null) return "Hasn't opened this game";
  return p.pending ? `${p.pending} to sync` : "In sync";
}

async function pickGame(id: number) {
  menu.value = "";
  await selectGame(id);
}

const nav = [
  { to: "/", label: "Mods", icon: Layers },
  { to: "/activity", label: "Activity", icon: History },
];
</script>

<template>
  <div class="shell">
    <template v-if="ready && setupDone()">
      <header class="topbar" :class="{ scrolled }">
        <div class="row" style="gap: 14px; min-width: 0">
          <RouterLink to="/" class="brand" aria-label="Mods Manager, go to Mods">
            <span class="brand-tile"><img :src="markWhiteUrl" alt="" draggable="false" /></span>
            <img class="brand-mark" :src="markUrl" alt="" draggable="false" />
            mods
          </RouterLink>
          <div v-if="store.games" class="menu-wrap">
            <button class="game-switch" :aria-expanded="menu === 'games'" @click="menu = menu === 'games' ? '' : 'games'">
              <Gamepad2 :size="16" />
              <span class="ellipsis">{{ game?.name ?? "No game yet" }}</span>
              <ChevronDown :size="14" class="muted" />
            </button>
            <div v-if="menu === 'games'" class="account-menu game-menu" role="menu">
              <button v-for="g in store.games" :key="g.id" class="account-menu-item" role="menuitemradio" :aria-checked="g.id === game?.id" @click="pickGame(g.id)">
                <Gamepad2 /> <span class="ellipsis" style="flex: 1">{{ g.name }}</span>
                <span class="hint num">{{ g.mods }}</span>
                <Check v-if="g.id === game?.id" :size="14" style="color: var(--tint)" />
              </button>
              <div class="menu-sep" />
              <button v-if="game" class="account-menu-item" role="menuitem" @click="(menu = ''), (gameDialog.edit = game)"><Pencil /> Edit {{ game.name }}</button>
              <button class="account-menu-item" role="menuitem" @click="(menu = ''), (gameDialog.add = true)"><Plus /> Add a game</button>
            </div>
          </div>
        </div>
        <nav class="nav">
          <RouterLink v-for="item in nav" :key="item.to" :to="item.to" :class="{ active: $route.path === item.to }">
            <component :is="item.icon" :size="18" />
            {{ item.label }}
          </RouterLink>
        </nav>
        <div class="topbar-right">
          <div class="menu-wrap">
            <button class="people" :aria-expanded="menu === 'people'" :title="`${others.length} other ${others.length === 1 ? 'person' : 'people'}`" @click="menu = menu === 'people' ? '' : 'people'">
              <span class="faces">
                <span v-for="p in others.slice(0, 4)" :key="p.name" class="avatar face">{{ p.name.slice(0, 1) }}<span class="presence" :class="{ on: p.online }" /></span>
                <span v-if="others.length > 4" class="avatar face more">+{{ others.length - 4 }}</span>
                <span v-if="!others.length" class="avatar face more"><UserPlus :size="14" /></span>
              </span>
              <span class="friend-state" :class="{ behind }">{{ !others.length ? "Just you" : behind ? `${behind} behind` : "All in sync" }}</span>
            </button>
            <div v-if="menu === 'people'" class="account-menu people-menu" role="dialog" aria-label="People">
              <div class="account-menu-head"><strong>People</strong><span class="hint">{{ game ? `In ${game.name}` : "On this database" }}</span></div>
              <div v-for="p in people" :key="p.name" class="person">
                <span class="avatar">{{ p.name.slice(0, 1) }}<span class="presence" :class="{ on: p.online }" /></span>
                <span class="stack" style="gap: 0; min-width: 0; flex: 1">
                  <strong class="ellipsis">{{ p.name }}<span v-if="p.me" class="hint"> (you)</span></strong>
                  <span class="hint">{{ p.online ? "Online" : `Seen ${ago(p.lastSeen)}` }}</span>
                </span>
                <span class="friend-state" :class="{ behind: p.pending, idle: p.pending == null }">{{ stateText(p) }}</span>
              </div>
              <div class="menu-sep" />
              <button class="account-menu-item" @click="(menu = ''), (inviting = true)"><UserPlus /> Invite someone</button>
            </div>
          </div>
          <button
            class="btn btn-ghost btn-icon queue-button"
            :class="{ busy: queueBusy, attention: !queueBusy && (queueWaiting || queueFailed) }"
            :title="queueBusy ? `${queueBusy} downloading or queued` : queueWaiting ? `${queueWaiting} waiting for a click on Nexus` : queueFailed ? `${queueFailed} failed` : 'Downloads'"
            aria-label="Downloads"
            @click="store.queueOpen = true"
          >
            <Loader2 v-if="queueBusy" class="spin" /><Download v-else />
            <span v-if="queueBusy || queueWaiting || queueFailed" class="queue-badge num">{{ queueBusy || queueWaiting || queueFailed }}</span>
          </button>
          <span class="live" :class="{ on: store.live }" :title="store.live ? 'Live: changes show up right away' : 'Not connected to the shared database'">
            <span class="run-dot" />{{ store.live ? "Live" : "Offline" }}
          </span>
          <button v-if="updater.available" class="update-badge" :disabled="updater.installing" :title="`Version ${updater.available.version} is ready`" @click="installUpdate">
            <Loader2 v-if="updater.installing" class="spin" :size="16" /><RefreshCw v-else :size="16" />
            {{ updater.installing ? (store.syncing ? "Waiting for the sync…" : updater.progress ? `Updating ${Math.round(updater.progress * 100)}%` : "Updating…") : "Restart to update" }}
          </button>
          <button
            v-else
            class="btn btn-ghost btn-icon"
            :title="updater.current ? `Check for updates (you have ${updater.current})` : 'Check for updates'"
            aria-label="Check for updates"
            :disabled="updater.checking"
            @click="checkForUpdate(true)"
          >
            <RefreshCw :class="{ spin: updater.checking }" />
          </button>
          <button class="btn btn-ghost btn-icon" title="Settings" aria-label="Settings" @click="openSettings()"><SettingsIcon /></button>
        </div>
      </header>
      <div class="body">
        <main class="content" @scroll="scrolled = ($event.target as HTMLElement).scrollTop > 4">
          <RouterView v-slot="{ Component, route }">
            <component :is="Component" :key="`${route.fullPath}:${currentGame()}`" />
          </RouterView>
        </main>
      </div>
      <SettingsDialog v-if="settingsDialog.open" />
      <GameDialog v-if="gameDialog.add" @close="gameDialog.add = false" />
      <GameDialog v-if="gameDialog.edit" :game="gameDialog.edit" @close="gameDialog.edit = null" />
      <GameDialog v-if="gameDialog.paths" :game="gameDialog.paths" paths-only @close="gameDialog.paths = null" />
      <InviteDialog v-if="inviting" @close="inviting = false" />
      <QueuePanel v-if="store.queueOpen" />
    </template>
    <SetupPage v-else-if="ready" />
    <Toast />
  </div>
</template>
