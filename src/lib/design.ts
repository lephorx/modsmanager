export type Design = "modern" | "classic";

export function currentDesign(): Design {
  return document.documentElement.dataset.design === "classic" ? "classic" : "modern";
}

/** Same as the boot script in index.html, cross-faded when possible. */
export function setDesign(design: Design) {
  localStorage.setItem("mm-design", design);
  const apply = () => {
    const root = document.documentElement;
    const dark = design === "classic" || matchMedia("(prefers-color-scheme: dark)").matches;
    root.dataset.design = design;
    root.classList.toggle("dark", dark);
    root.style.colorScheme = dark ? "dark" : "light";
  };
  const doc = document as Document & { startViewTransition?: (cb: () => void) => unknown };
  if (doc.startViewTransition && !matchMedia("(prefers-reduced-motion: reduce)").matches) doc.startViewTransition(apply);
  else apply();
}
