/* Ported from js/core/easing.js — behavior-preserving. */
import type { PMRegistry } from '../registry';

export function install(PM: PMRegistry): void {
/** Newton + bisect cubic-bezier solver, cached per curve. */
const cache = new Map<string, any>();
function bezier(x1: number, y1: number, x2: number, y2: number): any {
  const key = x1 + ',' + y1 + ',' + x2 + ',' + y2;
  let f = cache.get(key);
  if (f) return f;
  if (x1 === y1 && x2 === y2) { f = (t: number) => t; cache.set(key, f); return f; }
  const cx = 3 * x1, bx = 3 * (x2 - x1) - cx, ax = 1 - cx - bx;
  const cy = 3 * y1, by = 3 * (y2 - y1) - cy, ay = 1 - cy - by;
  const sx = (t: number) => ((ax * t + bx) * t + cx) * t;
  const sy = (t: number) => ((ay * t + by) * t + cy) * t;
  const dx = (t: number) => (3 * ax * t + 2 * bx) * t + cx;
  f = (x: number) => {
    if (x <= 0) return 0; if (x >= 1) return 1;
    let t = x;
    for (let i = 0; i < 6; i++) {
      const d = sx(t) - x;
      if (Math.abs(d) < 1e-6) return sy(t);
      const g = dx(t);
      if (Math.abs(g) < 1e-6) break;
      t -= d / g;
    }
    let lo = 0, hi = 1; t = x;
    for (let i = 0; i < 24; i++) {
      const d = sx(t) - x;
      if (Math.abs(d) < 1e-6) break;
      d > 0 ? (hi = t) : (lo = t);
      t = (lo + hi) / 2;
    }
    return sy(t);
  };
  cache.set(key, f);
  return f;
}

/* Presets are stored as bezier handle pairs so the graph editor can show them. */
const PRESETS: Record<string, number[]> = {
  linear:      [0, 0, 1, 1],
  ease:        [.25, .1, .25, 1],
  easeIn:      [.42, 0, 1, 1],
  easeOut:     [0, 0, .58, 1],
  easeInOut:   [.42, 0, .58, 1],
  quadIn:      [.11, 0, .5, 0],      quadOut:  [.5, 1, .89, 1],      quadInOut: [.45, 0, .55, 1],
  cubicIn:     [.32, 0, .67, 0],     cubicOut: [.33, 1, .68, 1],     cubicInOut: [.65, 0, .35, 1],
  quartIn:     [.5, 0, .75, 0],      quartOut: [.25, 1, .5, 1],      quartInOut: [.76, 0, .24, 1],
  expoIn:      [.7, 0, .84, 0],      expoOut:  [.16, 1, .3, 1],      expoInOut: [.87, 0, .13, 1],
  circIn:      [.55, 0, 1, .45],     circOut:  [0, .55, .45, 1],     circInOut: [.85, 0, .15, 1],
  backIn:      [.36, 0, .66, -.56],  backOut:  [.34, 1.56, .64, 1],  backInOut: [.68, -.6, .32, 1.6],
  /* Powermove house curve — decisive entry, long settle. */
  power:       [.62, .05, 0, 1],
  snap:        [.9, 0, .1, 1],
  glide:       [.16, .84, .24, 1],
};

const Ease: any = {
  bezier,
  PRESETS,
  names: Object.keys(PRESETS),
  fn(name: string) { const p = PRESETS[name] || PRESETS.linear!; return bezier(p[0]!, p[1]!, p[2]!, p[3]!); },
  /** Match a handle pair to the closest preset name (for UI display). */
  nameOf(eo: number[], ei: number[]) {
    for (const k in PRESETS) {
      const p = PRESETS[k]!;
      if (Math.abs(p[0]! - eo[0]!) < .02 && Math.abs(p[1]! - eo[1]!) < .02 &&
          Math.abs(p[2]! - ei[0]!) < .02 && Math.abs(p[3]! - ei[1]!) < .02) return k;
    }
    return 'custom';
  },
  handles(name: string) { const p = PRESETS[name] || PRESETS.linear!; return { eo: [p[0], p[1]], ei: [p[2], p[3]] }; },
};

/* Springs simulate at a fixed internal timestep so the trajectory is identical at any fps. */
const SDT = 1 / 480;
const springCache = new Map<string, Float32Array>();
Ease.spring = (t: number, { damping = 18, stiffness = 180, mass = 1 }: any = {}) => {
  if (t <= 0) return 0;
  const key = damping + '|' + stiffness + '|' + mass;
  let tbl = springCache.get(key);
  if (!tbl) {
    tbl = new Float32Array(Math.ceil(6 / SDT));
    let x = 0, v = 0;
    for (let i = 0; i < tbl.length; i++) {
      const a = (-stiffness * (x - 1) - damping * v) / mass;
      v += a * SDT; x += v * SDT; tbl[i] = x;
    }
    springCache.set(key, tbl);
  }
  const i = Math.min(tbl.length - 1, Math.floor(t / SDT));
  return tbl[i];
};
Ease.springDuration = (cfg: any) => {
  const { damping = 18, stiffness = 180, mass = 1 } = cfg || {};
  let x = 0, v = 0;
  for (let i = 1; i < 10 / SDT; i++) {
    const a = (-stiffness * (x - 1) - damping * v) / mass;
    v += a * SDT; x += v * SDT;
    if (Math.abs(x - 1) < .002 && Math.abs(v) < .002) return i * SDT;
  }
  return 10;
};

PM.Ease = Ease;
}
