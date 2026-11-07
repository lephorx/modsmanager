<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { RouterLink, RouterView } from "vue-router";
import { History, Layers, Loader2, RefreshCw, Settings as SettingsIcon } from "lucide-vue-next";
import markUrl from "./assets/mark.svg";
import markWhiteUrl from "./assets/mark-white.svg";
import Toast from "./components/AppToast.vue";
import SettingsDialog from "./components/SettingsDialog.vue";
import SetupPage from "./pages/SetupPage.vue";
import { settingsDialog, openSettings } from "./lib/settingsDialog";
import { ONLINE_MS, loadSettings, refresh, setupDone, startEvents, store } from "./lib/store";
import { ago } from "./lib/format";
import { checkForUpdate, installUpdate, startUpdateChecks, updater } from "./lib/updater";

const scrolled = ref(false);
const ready = ref(false);

onMounted(async () => {
  await loadSettings();
  ready.value = true;
  await startEvents();
  void startUpdateChecks();
  await refresh();
});

/** Everyone else on the list: online or not, and whether they match it. */
const friends = computed(() => {
  const o = store.overview;
  if (!o) return [];
  return o.members
    .filter((m) => m.name !== o.me)
    .map((m) => ({ ...m, online: Date.now() - new Date(m.lastSeen).getTime() < ONLINE_MS }));
});

const nav = [
  { to: "/", label: "Mods", icon: Layers },
  { to: "/activity", label: "Activity", icon: History },
];
</script>

<template>
  <div class="shell">
    <template v-if="ready && setupDone()">
      <header class="topbar" :class="{ scrolled }">
        <div>
          <RouterLink to="/" class="brand" aria-label="Mods Manager, go to Mods">
            <span class="brand-tile"><img :src="markWhiteUrl" alt="" draggable="false" /></span>
            <img class="brand-mark" :src="markUrl" alt="" draggable="false" />
            mods
          </RouterLink>
        </div>
        <nav class="nav">
          <RouterLink v-for="item in nav" :key="item.to" :to="item.to" :class="{ active: $route.path === item.to }">
            <component :is="item.icon" :size="18" />
            {{ item.label }}
          </RouterLink>
        </nav>
        <div class="topbar-right">
          <span
            v-for="f in friends"
            :key="f.name"
            class="friend"
            :title="`${f.name}: ${f.online ? 'online' : `last seen ${ago(f.lastSeen)}`} · ${f.pending ? `${f.pending} changes to sync` : 'matches the list'}`"
          >
            <span class="avatar">{{ f.name.slice(0, 1) }}<span class="presence" :class="{ on: f.online }" /></span>
            <span class="friend-text">
              <strong class="ellipsis">{{ f.name }}</strong>
              <span class="friend-state" :class="{ behind: f.pending }">{{ f.pending ? `${f.pending} to sync` : "In sync" }}</span>
            </span>
          </span>
          <span class="live" :class="{ on: store.live }" :title="store.live ? 'Live: changes show up right away' : 'Not connected to the shared list'">
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
            <component :is="Component" :key="route.fullPath" />
          </RouterView>
        </main>
      </div>
      <SettingsDialog v-if="settingsDialog.open" />
    </template>
    <SetupPage v-else-if="ready" />
    <Toast />
  </div>
</template>
