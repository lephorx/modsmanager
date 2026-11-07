import { createRouter, createWebHashHistory } from "vue-router";
import ModsPage from "./pages/ModsPage.vue";
import ActivityPage from "./pages/ActivityPage.vue";

export const router = createRouter({
  history: createWebHashHistory(),
  routes: [
    { path: "/", component: ModsPage },
    { path: "/activity", component: ActivityPage },
  ],
});
