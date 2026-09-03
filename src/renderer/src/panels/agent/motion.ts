/* House motion curve for the agent feed: cubic-bezier(0.625, 0.05, 0, 1).

   This is a JS easing because the two transitions below interpolate height and
   blur — properties Svelte drives from a `css` string frame by frame. The
   curve is intentionally the supermove one rather than `--ease`: it is a long
   slow-out that reads as content settling, not as a control responding. */

export interface MotionParams {
  duration?: number;
}

export interface MotionConfig {
  duration: number;
  /** Per-node offset; used by the word sweep, absent on the older transitions. */
  delay?: number;
  easing: (t: number) => number;
  css: (t: number, u: number) => string;
}

export function cubicBezier(x1: number, y1: number, x2: number, y2: number): (t: number) => number {
  const cx = 3 * x1;
  const bx = 3 * (x2 - x1) - cx;
  const ax = 1 - cx - bx;
  const cy = 3 * y1;
  const by = 3 * (y2 - y1) - cy;
  const ay = 1 - cy - by;

  const sampleX = (t: number): number => ((ax * t + bx) * t + cx) * t;
  const sampleY = (t: number): number => ((ay * t + by) * t + cy) * t;
  const sampleDX = (t: number): number => (3 * ax * t + 2 * bx) * t + cx;

  function solveX(x: number): number {
    let t = x;
    for (let i = 0; i < 8; i++) {
      const d = sampleX(t) - x;
      if (Math.abs(d) < 1e-6) return t;
      const dx = sampleDX(t);
      if (Math.abs(dx) < 1e-6) break;
      t -= d / dx;
    }
    let lo = 0;
    let hi = 1;
    t = x;
    for (let i = 0; i < 20; i++) {
      const d = sampleX(t) - x;
      if (Math.abs(d) < 1e-6) break;
      if (d > 0) hi = t;
      else lo = t;
      t = (lo + hi) / 2;
    }
    return t;
  }

  return (t: number) => sampleY(solveX(t));
}

export const motionEase = cubicBezier(0.625, 0.05, 0, 1);

/* Reduced motion is a hard opt-out, not a shortened duration: a 0ms
   transition still runs Svelte's bookkeeping but paints in one frame. */
export function prefersReducedMotion(): boolean {
  return typeof window !== 'undefined'
    && typeof window.matchMedia === 'function'
    && window.matchMedia('(prefers-reduced-motion: reduce)').matches;
}

/* Word reveal: long, soft fade (+ a touch of blur) so many words are
   mid-reveal at once — reads as a flowing wave rather than discrete typing. */
export function wordIn(_node: Element, { duration = 620 }: MotionParams = {}): MotionConfig {
  return {
    duration: prefersReducedMotion() ? 0 : duration,
    easing: motionEase,
    css: (t, u) => `opacity:${t}; filter: blur(${u * 2.5}px);`
  };
}

/* Run halo exit: the glow around the answered prompt settles out over half a
   second rather than blinking off the instant the run lands. The easing is
   linear on purpose — the house curve is a slow-out, and on a fade it dumps
   most of the light in the first sixth of the duration, which reads as a blink
   with a tail. The node — and with it the WebGPU canvas — goes when this ends. */
export function glowFade(_node: Element, { duration = 520 }: MotionParams = {}): MotionConfig {
  return {
    duration: prefersReducedMotion() ? 0 : duration,
    easing: (t) => t,
    css: (t) => `opacity:${t};`
  };
}

/* Step reveal: collapse-down height + fade. The row's own height is measured
   once at transition start, so a wrapped multi-line thought opens to its real
   height instead of a guessed one. */
export function stepIn(node: Element, { duration = 260 }: MotionParams = {}): MotionConfig {
  const h = (node as HTMLElement).offsetHeight || 0;
  return {
    duration: prefersReducedMotion() ? 0 : duration,
    easing: motionEase,
    css: (t) => `opacity:${t}; height:${t * h}px; overflow:hidden;`
  };
}

/* ---------------------------------------------------------------------------
   Scritto roll (scrit.to, MIT) — ported, not depended on.

   Scritto is SwiftUI's `.numericText` as a web component: it diffs a value and
   rolls the glyphs that changed. Its own README rules out the case we have —
   "if you need a paragraph of text to reflow, this is the wrong tool" — because
   it gives every glyph its own non-wrapping box. Agent prose is exactly a
   reflowing paragraph, so what we take is the motion signature, applied per
   WORD over the tokens `toRichWords` already emits. Words keep their kerning,
   the paragraph keeps its wrapping, and nothing ships.

   Constants below are Scritto 0.1.0's enter animation, verbatim: rise 0.35em,
   scale 0.6, blur 0.1em, rotate 2deg, over 550ms on its default spring. */

const SCRITTO = { y: 0.35, scale: 0.6, blur: 0.1, rotate: 2 } as const;

/* Scritto's default `transition.easing`, a CSS `linear()` stop list. Svelte
   wants a JS easing (it samples `css` into keyframes itself), so the stops are
   resampled here rather than handed to the browser. The tail above 1 is the
   spring's overshoot and is meant to be there. */
const SCRITTO_STOPS = [
  0, 0.1052, 0.3155, 0.532, 0.7112, 0.8414, 0.9265, 0.9765,
  1.0023, 1.013, 1.0151, 1.0133, 1.01, 1.0068, 1.0041, 1.0022, 1.001, 1
];

export function scrittoEase(t: number): number {
  if (t <= 0) return SCRITTO_STOPS[0]!;
  if (t >= 1) return SCRITTO_STOPS[SCRITTO_STOPS.length - 1]!;
  const span = (SCRITTO_STOPS.length - 1) * t;
  const i = Math.floor(span);
  const a = SCRITTO_STOPS[i]!;
  const b = SCRITTO_STOPS[i + 1]!;
  return a + (b - a) * (span - i);
}

/* Scritto sweeps its glyphs across 0.3 of the duration. A streamed word has no
   batch to sweep against — it arrives alone — so the sweep is measured per DOM
   flush instead of per paragraph: one word appended this tick starts at once,
   twenty words landing together fan out. */
const SWEEP_MS = 0.3 * 550;
const SWEEP_DECAY = 8;

/* How long a whole reveal takes to settle: the roll, plus the longest sweep
   offset any word in the batch can be given, plus a frame of slack. */
export const WORD_REVEAL_SETTLE_MS = 550 + SWEEP_MS + 80;

let batchIndex = 0;
let batchScheduled = false;

/* Reset on the microtask after the flush that created these nodes, so "one
   batch" means literally one Svelte update, however many words it added. */
export function nextInBatch(): number {
  if (!batchScheduled) {
    batchScheduled = true;
    queueMicrotask(() => {
      batchIndex = 0;
      batchScheduled = false;
    });
  }
  return batchIndex++;
}

/* Asymptotic rather than linear: every word is offset from the one before it,
   but a 400-word replayed message still finishes its wave in SWEEP_MS instead
   of trickling for half a minute. */
export function sweepDelay(index: number): number {
  return SWEEP_MS * (1 - Math.exp(-index / SWEEP_DECAY));
}

/* Word roll-in. `display:inline-block` is set for the duration only — an inline
   box takes no transform, and leaving it on would change how the line breaks
   after the motion is over. Svelte strips the whole declaration at t=1. */
export function wordRollIn(_node: Element, { duration = 550 }: MotionParams = {}): MotionConfig {
  if (prefersReducedMotion()) {
    return { duration: 0, delay: 0, easing: scrittoEase, css: () => '' };
  }
  return {
    duration,
    delay: sweepDelay(nextInBatch()),
    easing: scrittoEase,
    css: (t, u) => {
      const scale = SCRITTO.scale + (1 - SCRITTO.scale) * t;
      return `display:inline-block; opacity:${Math.min(t, 1)};`
        + ` filter:blur(${u * SCRITTO.blur}em);`
        + ` transform:translateY(${u * SCRITTO.y}em) scale(${scale}) rotateZ(${u * SCRITTO.rotate}deg);`;
    }
  };
}
