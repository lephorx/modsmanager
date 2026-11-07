<script setup lang="ts">
import { ref } from "vue";
import { ArrowRight, CircleCheck, Database, FolderOpen, KeyRound, Loader2, UserRound } from "lucide-vue-next";
import { open } from "@tauri-apps/plugin-dialog";
import markWhiteUrl from "@/assets/mark-white.svg";
import { api, type Settings } from "@/lib/api";
import { refresh, store } from "@/lib/store";
import { errorText } from "@/lib/toast";

/** First start: who you are, the shared database, mod.io and BONELAB's folders. */
const s = ref<Settings>({ ...store.settings! });
const testing = ref(false);
const tested = ref(false);
const error = ref("");
const saving = ref(false);

async function test() {
  testing.value = true;
  error.value = "";
  tested.value = false;
  try {
    await api.testConnection(s.value.databaseUrl);
    tested.value = true;
  } catch (e) {
    error.value = errorText(e);
  } finally {
    testing.value = false;
  }
}

async function browse(key: "modsDir" | "gameDir") {
  const dir = await open({ directory: true, defaultPath: s.value[key] || undefined });
  if (typeof dir === "string") s.value[key] = dir;
}

async function finish() {
  error.value = "";
  if (!s.value.name.trim()) return (error.value = "Enter your name");
  if (!s.value.databaseUrl.trim()) return (error.value = "Enter the shared database address");
  if (!s.value.modsDir.trim()) return (error.value = "Choose BONELAB's Mods folder");
  saving.value = true;
  try {
    await api.testConnection(s.value.databaseUrl);
    await api.saveSettings(s.value);
    store.settings = { ...s.value };
    await refresh();
  } catch (e) {
    error.value = errorText(e);
  } finally {
    saving.value = false;
  }
}
</script>

<template>
  <div class="setup">
    <div class="setup-logo">
      <span class="brand-tile big"><img :src="markWhiteUrl" alt="" /></span>
      <span class="brand-word">mods</span>
    </div>
    <form class="card setup-card stack" @submit.prevent="finish">
      <div class="stack" style="gap: 4px; text-align: center">
        <h1 class="setup-title">Set up Mods Manager</h1>
        <p class="hint">You and your friend connect to the same list, so you always have the exact same BONELAB mods.</p>
      </div>
      <div class="field">
        <label for="s-name">Your name</label>
        <label class="input-icon"><UserRound /><input id="s-name" v-model="s.name" class="input" placeholder="Shown to your friend" autofocus /></label>
      </div>
      <div class="field">
        <label for="s-db">Shared database</label>
        <div class="row" style="gap: 8px">
          <label class="input-icon" style="flex: 1">
            <Database />
            <input id="s-db" v-model="s.databaseUrl" class="input mono" placeholder="postgres://user:password@host:5432/mods" @input="tested = false" />
          </label>
          <button type="button" class="btn" :disabled="!s.databaseUrl.trim() || testing" @click="test">
            <Loader2 v-if="testing" class="spin" /><CircleCheck v-else-if="tested" style="color: var(--status-green)" /> Test
          </button>
        </div>
        <span class="hint">A Postgres database you both can reach; the same address on both computers. The tables are created for you.</span>
      </div>
      <div class="field">
        <label for="s-key">mod.io API key</label>
        <label class="input-icon"><KeyRound /><input id="s-key" v-model="s.modioApiKey" class="input mono" placeholder="From mod.io → Account → API access" /></label>
      </div>
      <div class="field">
        <label for="s-mods">BONELAB Mods folder</label>
        <div class="row" style="gap: 8px">
          <input id="s-mods" v-model="s.modsDir" class="input mono" placeholder="…\AppData\LocalLow\Stress Level Zero\BONELAB\Mods" />
          <button type="button" class="btn btn-icon" title="Browse" aria-label="Browse" @click="browse('modsDir')"><FolderOpen /></button>
        </div>
      </div>
      <div class="field">
        <label for="s-game">BONELAB game folder <span class="hint">(for MelonLoader code mods)</span></label>
        <div class="row" style="gap: 8px">
          <input id="s-game" v-model="s.gameDir" class="input mono" placeholder="…\steamapps\common\BONELAB" />
          <button type="button" class="btn btn-icon" title="Browse" aria-label="Browse" @click="browse('gameDir')"><FolderOpen /></button>
        </div>
      </div>
      <p v-if="error" class="error-text selectable">{{ error }}</p>
      <button class="btn btn-primary btn-lg" type="submit" :disabled="saving" style="width: 100%">
        <Loader2 v-if="saving" class="spin" /> Continue <ArrowRight v-if="!saving" />
      </button>
    </form>
  </div>
</template>
