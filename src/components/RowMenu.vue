<script setup lang="ts">
import { nextTick, onUnmounted, ref, type Component } from "vue";
import { EllipsisVertical } from "lucide-vue-next";

export interface RowMenuItem {
  label: string;
  icon: Component;
  danger?: boolean;
  action: () => void;
}

/** A ⋮ button with a menu of actions; the menu floats over everything, so a scrolling list doesn't cut it off. */
const props = defineProps<{ items: RowMenuItem[]; label: string }>();
const open = ref(false);
const button = ref<HTMLElement | null>(null);
const pos = ref({ top: 0, right: 0 });

function place() {
  const r = button.value?.getBoundingClientRect();
  if (!r) return;
  // above the button when it's near the bottom of the window
  const below = window.innerHeight - r.bottom > 48 * props.items.length + 24;
  pos.value = { top: below ? r.bottom + 6 : r.top - 6 - (40 * props.items.length + 12), right: window.innerWidth - r.right };
}

function close(e?: Event) {
  if (e && e.target instanceof Element && (e.target.closest(".row-menu") || button.value?.contains(e.target))) return;
  open.value = false;
  window.removeEventListener("mousedown", close, true);
  window.removeEventListener("scroll", onScroll, true);
  window.removeEventListener("keydown", onKey);
}
const onScroll = () => close();
// after the dialog's own Escape handler, which leaves the dialog open while a menu is
const onKey = (e: KeyboardEvent) => e.key === "Escape" && close();

async function toggle() {
  if (open.value) return close();
  place();
  open.value = true;
  await nextTick();
  window.addEventListener("mousedown", close, true);
  window.addEventListener("scroll", onScroll, true);
  window.addEventListener("keydown", onKey);
}

function run(item: RowMenuItem) {
  close();
  item.action();
}
onUnmounted(() => close());
</script>

<template>
  <button ref="button" class="btn btn-ghost btn-icon" :title="label" :aria-label="label" aria-haspopup="menu" :aria-expanded="open" @click="toggle"><EllipsisVertical /></button>
  <Teleport to="body">
    <div v-if="open" class="account-menu row-menu" role="menu" :style="{ position: 'fixed', top: `${pos.top}px`, right: `${pos.right}px` }">
      <button v-for="i in items" :key="i.label" class="btn btn-ghost account-menu-item" :class="{ danger: i.danger }" role="menuitem" @click="run(i)">
        <component :is="i.icon" :size="16" />{{ i.label }}
      </button>
    </div>
  </Teleport>
</template>

<style scoped>
.row-menu {
  z-index: 200;
  width: auto;
  min-width: 220px;
}
.row-menu .account-menu-item {
  justify-content: flex-start;
}
.danger {
  color: var(--destructive);
}
</style>
