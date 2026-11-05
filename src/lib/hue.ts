// The accent's fade from one song's hues to the next (D14, D17). A CSS
// transition on `--h` would restyle the whole document every frame for its
// 1.2 s, since every colour inherits from it. The fade is written from here
// on the same time and curve, in about 20 steps rather than 72: the curve
// does most of its change early, so the first 300 ms are written every
// 33 ms and the rest every 100 ms, which keeps each step small (about 20°
// at most on a half-turn) at a fraction of the work.

/** How long the fade takes, and how often it is written: often while the
 *  curve moves fast, less after. */
const FADE_MS = 1200;
const FAST_MS = 300;
const FAST_STEP_MS = 33;
const STEP_MS = 100;

/** When (ms into the fade) each step is written; the last is the end. */
export function fadeTimes(): number[] {
  const out: number[] = [];
  for (let t = 16; t < FADE_MS; t += t < FAST_MS ? FAST_STEP_MS : STEP_MS) out.push(t);
  out.push(FADE_MS);
  return out;
}

/** `cubic-bezier(x1, y1, x2, y2)` as CSS evaluates it: y for a time x in 0..1. */
export function bezier(x1: number, y1: number, x2: number, y2: number) {
  const at = (a: number, b: number, t: number) => 3 * a * t * (1 - t) ** 2 + 3 * b * t * t * (1 - t) + t ** 3;
  const slope = (a: number, b: number, t: number) => 3 * a * (1 - t) ** 2 + 6 * (b - a) * t * (1 - t) + 3 * (1 - b) * t * t;
  return (x: number) => {
    if (x <= 0) return 0;
    if (x >= 1) return 1;
    // Newton's method from x, then bisection if it didn't settle.
    let t = x;
    for (let i = 0; i < 8; i++) {
      const d = at(x1, x2, t) - x;
      if (Math.abs(d) < 1e-6) return at(y1, y2, t);
      const s = slope(x1, x2, t);
      if (Math.abs(s) < 1e-6) break;
      t -= d / s;
    }
    let lo = 0;
    let hi = 1;
    t = x;
    for (let i = 0; i < 30; i++) {
      const v = at(x1, x2, t);
      if (Math.abs(v - x) < 1e-6) break;
      if (v < x) lo = t;
      else hi = t;
      t = (lo + hi) / 2;
    }
    return at(y1, y2, t);
  };
}

/** `--ease` in global.css. */
export const ease = bezier(0.2, 0.7, 0.1, 1);

/** The hues between `from` and `to` at a time `t` (0..1) into the fade:
 *  straight across the numbers, as the CSS transition of a `<number>` went. */
export function hueAt(from: [number, number], to: [number, number], t: number): [number, number] {
  const e = ease(t);
  return [from[0] + (to[0] - from[0]) * e, from[1] + (to[1] - from[1]) * e];
}

// What is on screen now: @property's initial values until the first song.
let shown: [number, number] = [205, 250];
let timer: ReturnType<typeof setTimeout> | undefined;

function write(h: [number, number]) {
  shown = h;
  const root = document.documentElement.style;
  root.setProperty("--h", h[0].toFixed(1));
  root.setProperty("--h2", h[1].toFixed(1));
}

/** Fades the accent to `to`; jumps with reduced motion or a hidden window.
 *  A new fade starts from wherever the last one had got to. */
export function fadeHue(to: [number, number]) {
  clearTimeout(timer);
  timer = undefined;
  if (shown[0] === to[0] && shown[1] === to[1]) return;
  if (document.hidden || matchMedia("(prefers-reduced-motion: reduce)").matches) {
    write(to);
    return;
  }
  const from = shown;
  const start = performance.now();
  const step = () => {
    const ms = performance.now() - start;
    if (ms >= FADE_MS || document.hidden) {
      timer = undefined;
      write(to);
      return;
    }
    write(hueAt(from, to, ms / FADE_MS));
    timer = setTimeout(step, ms < FAST_MS ? FAST_STEP_MS : STEP_MS);
  };
  // The first step at once (the next frame), not a whole step later.
  timer = setTimeout(step, 16);
}
