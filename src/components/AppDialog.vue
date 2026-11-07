<script setup lang="ts">
import { onMounted, onUnmounted } from "vue";
import { X } from "lucide-vue-next";

withDefaults(defineProps<{ title: string; width?: number }>(), { width: 460 });
const emit = defineEmits<{ close: [] }>();

function onKey(e: KeyboardEvent) {
  if (e.key === "Escape") emit("close");
}
onMounted(() => window.addEventListener("keydown", onKey));
onUnmounted(() => window.removeEventListener("keydown", onKey));
</script>

<template>
  <!-- In <body>: the page's entrance animation would otherwise clip the fixed backdrop. -->
  <Teleport to="body">
    <div class="backdrop" @mousedown.self="emit('close')">
      <div class="dialog" :style="{ maxWidth: `${width}px` }" role="dialog" :aria-label="title">
        <div class="dialog-head">
          <h2>{{ title }}</h2>
          <button class="dialog-close" aria-label="Close" title="Close" @click="emit('close')"><X /></button>
        </div>
        <div class="dialog-body">
          <slot />
        </div>
      </div>
    </div>
  </Teleport>
</template>
