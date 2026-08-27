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
