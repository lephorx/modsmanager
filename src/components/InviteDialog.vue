<script setup lang="ts">
import { onMounted, ref } from "vue";
import { Check, Copy, Loader2, TriangleAlert } from "lucide-vue-next";
import AppDialog from "./AppDialog.vue";
import SecretInput from "./SecretInput.vue";
import { api } from "@/lib/api";
import { currentGame, store } from "@/lib/store";
import { errorText } from "@/lib/toast";

/** A code that sets up a friend's app: the shared database and the game you're looking at. */
const emit = defineEmits<{ close: [] }>();
const code = ref("");
const error = ref("");
const copied = ref(false);
const gameName = store.games?.find((g) => g.id === currentGame())?.name;

onMounted(async () => {
  try {
    code.value = await api.inviteCode(currentGame());
  } catch (e) {
    error.value = errorText(e);
  }
});

async function copy() {
  await navigator.clipboard.writeText(code.value);
  copied.value = true;
  setTimeout(() => (copied.value = false), 2000);
}
</script>

<template>
  <AppDialog title="Invite someone" :width="520" @close="emit('close')">
    <p>
      Send this code to a friend. They paste it when they set up Mods Manager and land on
      <strong>{{ gameName ?? "the shared list" }}</strong>, with every game you share. Anyone can join: there's no limit on people.
    </p>
    <p v-if="error" class="error-text selectable">{{ error }}</p>
    <div v-else-if="!code" class="row hint" style="gap: 8px"><Loader2 class="spin" /> Making the code…</div>
    <template v-else>
      <div class="row" style="gap: 8px">
        <SecretInput v-model="code" style="flex: 1" />
        <button class="btn btn-primary" @click="copy"><Check v-if="copied" /><Copy v-else /> {{ copied ? "Copied" : "Copy" }}</button>
      </div>
      <div class="inset row" style="gap: 10px; padding: 12px 14px; align-items: flex-start">
        <TriangleAlert :size="18" style="color: var(--status-orange); flex-shrink: 0; margin-top: 1px" />
        <span class="hint">The code contains the database address and password. Only send it to people you trust, in a private message.</span>
      </div>
    </template>
    <div class="dialog-footer">
      <button class="btn" @click="emit('close')">Done</button>
    </div>
  </AppDialog>
</template>
