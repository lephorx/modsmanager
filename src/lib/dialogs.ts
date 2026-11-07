import { reactive } from "vue";
import type { GameView } from "./api";

/** The game dialogs, openable from anywhere (top bar, Mods page, Settings). */
export const gameDialog = reactive({
  add: false,
  edit: null as GameView | null,
  /** only this computer's folders for the game (after joining someone's list) */
  paths: null as GameView | null,
});
