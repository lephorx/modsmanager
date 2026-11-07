import type { Directive } from "vue";

/**
 * `v-slider` keeps a pill under the selected item of a segmented tab bar and slides it
 * along, like the dashboard's tabs (CSCM components/ui/tabs.tsx): the leading edge reaches
 * the new tab first, so the pill briefly spans both, then the trailing edge catches up.
 * Modern look only (hidden in Classic, see app.css). Selected item: `.active`.
 *
 * The pill is three pieces (left cap, stretchable middle, right cap) moved only with
 * `transform`, so WebKit runs the animation on the compositor: it stays smooth while the
 * page is busy rendering the new panel. Animating `width` instead re-lays out the page on
 * every frame and stutters whenever the main thread is.
 */

const CAP = 12; // = the pill's corner radius
const DURATION = 520;
const EASE_IN = "cubic-bezier(0.45, 0, 0.55, 1)";
const EASE_OUT = "cubic-bezier(0.22, 1, 0.36, 1)";

interface Box {
  x: number;
  width: number;
}

interface State {
  root: HTMLSpanElement;
  left: HTMLSpanElement;
  mid: HTMLSpanElement;
  right: HTMLSpanElement;
  current: HTMLElement | null;
  /** Where the pill rests (or is heading). */
  box: Box | null;
  started: number;
  from: Box | null;
  running: Animation[];
  sizes: ResizeObserver;
  mutations: MutationObserver;
}

const states = new WeakMap<HTMLElement, State>();

const capLeft = (b: Box) => `translateX(${b.x}px)`;
const middle = (b: Box) => `translateX(${b.x + CAP - 0.5}px) scaleX(${Math.max(b.width - 2 * CAP + 1, 0)})`;
const capRight = (b: Box) => `translateX(${b.x + b.width - CAP}px)`;

function apply(s: State, b: Box) {
  s.left.style.transform = capLeft(b);
  s.mid.style.transform = middle(b);
  s.right.style.transform = capRight(b);
}

/** The pill's box right now, mid-animation included (same easing as the keyframes). */
function presented(s: State): Box | null {
  if (!s.box) return null;
  if (!s.running.length || !s.from) return s.box;
  const t = Math.min((performance.now() - s.started) / DURATION, 1);
  // Close enough for re-targeting: read the middle piece's current matrix.
  const m = new DOMMatrixReadOnly(getComputedStyle(s.mid).transform);
  if (t >= 1 || m.isIdentity) return s.box;
  const width = m.a + 2 * CAP - 1;
  return { x: m.e - CAP + 0.5, width };
}

function place(list: HTMLElement, s: State) {
  const active = list.querySelector<HTMLElement>(":scope > .active");
  if (!active) {
    s.root.style.opacity = "0";
    s.current = null;
    return;
  }
  const to: Box = { x: active.offsetLeft, width: active.offsetWidth };
  s.root.style.opacity = "1";
  s.root.style.top = `${active.offsetTop}px`;
  s.root.style.height = `${active.offsetHeight}px`;

  const moved = s.current !== null && active !== s.current;
  const animate =
    moved && document.documentElement.dataset.design === "modern" && !matchMedia("(prefers-reduced-motion: reduce)").matches;
  const from = animate ? presented(s) : null;

  for (const a of s.running) a.cancel();
  s.running = [];
  apply(s, to);
  s.box = to;

  if (from && (from.x !== to.x || from.width !== to.width)) {
    const forward = to.x + to.width / 2 > from.x + from.width / 2;
    const stretched: Box = forward
      ? { x: from.x, width: to.x + to.width - from.x }
      : { x: to.x, width: from.x + from.width - to.x };
    const frames = (f: (b: Box) => string) => [
      { transform: f(from), easing: EASE_IN },
      { transform: f(stretched), offset: 0.42, easing: EASE_OUT },
      { transform: f(to) },
    ];
    s.from = from;
    s.started = performance.now();
    s.running = [
      s.left.animate(frames(capLeft), { duration: DURATION }),
      s.mid.animate(frames(middle), { duration: DURATION }),
      s.right.animate(frames(capRight), { duration: DURATION }),
    ];
    s.running[0]!.onfinish = () => (s.running = []);
  }
  if (moved && list.scrollWidth > list.clientWidth) active.scrollIntoView({ block: "nearest", inline: "nearest" });
  s.current = active;
}

export const slider: Directive<HTMLElement> = {
  mounted(list) {
    list.dataset.slider = "";
    const root = document.createElement("span");
    root.className = "slider-indicator";
    root.setAttribute("aria-hidden", "true");
    const left = document.createElement("span");
    const mid = document.createElement("span");
    const right = document.createElement("span");
    left.className = "slider-cap slider-cap-left";
    mid.className = "slider-mid";
    right.className = "slider-cap slider-cap-right";
    root.append(left, mid, right);
    list.prepend(root);

    const s: State = {
      root,
      left,
      mid,
      right,
      current: null,
      box: null,
      started: 0,
      from: null,
      running: [],
      sizes: new ResizeObserver(() => place(list, s)),
      mutations: new MutationObserver((records) => {
        // only selection changes, not the pill's own style updates
        if (records.some((r) => r.target !== root && !root.contains(r.target as Node))) place(list, s);
      }),
    };
    states.set(list, s);
    s.mutations.observe(list, { subtree: true, attributes: true, attributeFilter: ["class"] });
    s.sizes.observe(list);
    for (const child of list.children) if (child !== root) s.sizes.observe(child);
    place(list, s);
  },
  unmounted(list) {
    const s = states.get(list);
    if (!s) return;
    for (const a of s.running) a.cancel();
    s.mutations.disconnect();
    s.sizes.disconnect();
    states.delete(list);
  },
};
