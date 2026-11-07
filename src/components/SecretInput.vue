<script setup lang="ts">
import { ref, type Component } from "vue";
import { Eye, EyeOff } from "lucide-vue-next";

/** A field for secrets (database address, API key, token, invite code): hidden until you ask to see it. */
defineProps<{ id?: string; placeholder?: string; icon?: Component }>();
const model = defineModel<string>({ required: true });
const emit = defineEmits<{ done: [] }>();
const shown = ref(false);
</script>

<template>
  <span class="secret" :class="{ 'input-icon': icon }">
    <component :is="icon" v-if="icon" />
    <input
      :id="id"
      v-model="model"
      class="input mono"
      :type="shown ? 'text' : 'password'"
      :placeholder="placeholder"
      autocomplete="off"
      spellcheck="false"
      @blur="emit('done')"
      @keydown.enter="emit('done')"
    />
    <button
      type="button"
      class="secret-toggle"
      :title="shown ? 'Hide' : 'Show'"
      :aria-label="shown ? 'Hide' : 'Show'"
      :aria-pressed="shown"
      @mousedown.prevent
      @click="shown = !shown"
    >
      <EyeOff v-if="shown" :size="16" /><Eye v-else :size="16" />
    </button>
  </span>
</template>
