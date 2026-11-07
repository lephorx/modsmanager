import { reactive } from "vue";

type Kind = "success" | "error" | "info";

export const toastState = reactive<{ kind: Kind; text: string; id: number }>({ id: 0, text: "", kind: "info" });

let timer: ReturnType<typeof setTimeout> | undefined;
function show(kind: Kind, text: string) {
  Object.assign(toastState, { kind, text, id: toastState.id + 1 });
  clearTimeout(timer);
  timer = setTimeout(() => (toastState.text = ""), kind === "error" ? 7000 : 3500);
}

export const toast = {
  success: (t: string) => show("success", t),
  error: (t: string) => show("error", t),
  info: (t: string) => show("info", t),
};

export function errorText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}
