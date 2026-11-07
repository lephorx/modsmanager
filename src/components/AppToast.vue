<script setup lang="ts">
import { CircleCheck, Info, OctagonX } from "lucide-vue-next";
import { toastState } from "@/lib/toast";
</script>

<template>
  <div v-if="toastState.text" :key="toastState.id" class="toast" :class="toastState.kind" role="status">
    <CircleCheck v-if="toastState.kind === 'success'" />
    <OctagonX v-else-if="toastState.kind === 'error'" />
    <Info v-else />
    <span class="selectable">{{ toastState.text }}</span>
  </div>
</template>
