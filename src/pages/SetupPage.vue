<script setup lang="ts">
import { ref } from "vue";
import { ArrowRight, CircleCheck, Database, KeyRound, Loader2, Ticket, UserRound } from "lucide-vue-next";
import markWhiteUrl from "@/assets/mark-white.svg";
import SecretInput from "@/components/SecretInput.vue";
import { api, DB_PLACEHOLDER, type Settings } from "@/lib/api";
import { refresh, store } from "@/lib/store";
import { errorText } from "@/lib/toast";

/** First start: who you are and which shared database. Games and their folders come after. */
const s = ref<Settings>({ ...store.settings! });
const how = ref<"invite" | "database">("invite");
const invite = ref("");
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

async function finish() {
  error.value = "";
  if (!s.value.name.trim()) return (error.value = "Enter your name");
  saving.value = true;
  try {
    const next = { ...s.value, name: s.value.name.trim() };
    if (how.value === "invite") {
      if (!invite.value.trim()) throw new Error("Paste the invite code you got");
      const inv = await api.readInvite(invite.value);
      next.databaseUrl = inv.databaseUrl;
      if (inv.game != null) next.currentGame = inv.game;
    } else if (!next.databaseUrl.trim()) {
      throw new Error("Enter the shared database address");
    }
    await api.testConnection(next.databaseUrl);
    await api.saveSettings(next);
    store.settings = next;
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
        <p class="hint">Everyone on the same list gets exactly the same mods, for any game.</p>
      </div>
      <div class="field">
        <label for="s-name">Your name</label>
        <label class="input-icon"><UserRound /><input id="s-name" v-model="s.name" class="input" placeholder="Shown to the others" autofocus /></label>
      </div>

      <div v-slider class="tabs" role="tablist">
        <button type="button" :class="{ active: how === 'invite' }" role="tab" @click="how = 'invite'"><Ticket /> I have an invite</button>
        <button type="button" :class="{ active: how === 'database' }" role="tab" @click="how = 'database'"><Database /> Use my database</button>
      </div>

      <div v-if="how === 'invite'" class="field">
        <label for="s-invite">Invite code</label>
        <SecretInput id="s-invite" v-model="invite" :icon="Ticket" placeholder="MM1-…" />
        <span class="hint">Someone already using Mods Manager makes one under People → Invite someone.</span>
      </div>
      <div v-else class="field">
        <label for="s-db">Shared database</label>
        <div class="row" style="gap: 8px">
          <SecretInput id="s-db" v-model="s.databaseUrl" :icon="Database" :placeholder="DB_PLACEHOLDER" style="flex: 1" @update:model-value="tested = false" />
          <button type="button" class="btn" :disabled="!s.databaseUrl.trim() || testing" @click="test">
            <Loader2 v-if="testing" class="spin" /><CircleCheck v-else-if="tested" style="color: var(--status-green)" /> Test
          </button>
        </div>
        <span class="hint">A Postgres database everyone can reach. The tables are created for you; then invite the others.</span>
      </div>

      <div class="field">
        <label for="s-key">mod.io API key <span class="hint">(for games on mod.io)</span></label>
        <SecretInput id="s-key" v-model="s.modioApiKey" :icon="KeyRound" placeholder="From mod.io → Account → API access" />
      </div>
      <p v-if="error" class="error-text selectable">{{ error }}</p>
      <button class="btn btn-primary btn-lg" type="submit" :disabled="saving" style="width: 100%">
        <Loader2 v-if="saving" class="spin" /> Continue <ArrowRight v-if="!saving" />
      </button>
    </form>
  </div>
</template>
