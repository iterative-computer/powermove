/* Text animators. Each animator selects characters, words or lines and pushes
   them away from their resting pose by its property values, scaled by a
   per-unit weight in 0…1 (Back and Spring easing briefly overshoot it).

   - stagger: time-based. Every unit eases from the animated values to rest
     ("in") or from rest to them ("out"), one after another. No keyframes.
   - range:   After Effects style. Start/End/Offset choose the affected units;
     keyframe them to sweep the selection across the text.
   - wave:    a continuous ripple; the weight oscillates in −1…1.

   Animators without a `mode` predate these modes: they are range selectors
   whose unit lives in the `unit` channel. */

export type TextAnimatorMode = 'stagger' | 'range' | 'wave';
export type TextAnimatorUnit = 'characters' | 'words' | 'lines';
export type TextAnimatorOrder = 'forward' | 'reverse' | 'center' | 'edges' | 'random';
export type TextAnimatorShape = 'square' | 'rampUp' | 'rampDown' | 'triangle' | 'round';

/** Offsets are added at full weight; scale and opacity are target percentages. */
export const TEXT_ANIMATOR_PROPERTIES = {
  opacity: { label: 'Opacity', rest: 100, unit: '%', min: 0, max: 100, step: 1 },
  x: { label: 'Position X', rest: 0, unit: 'px', step: 1 },
  y: { label: 'Position Y', rest: 0, unit: 'px', step: 1 },
  scale: { label: 'Scale', rest: 100, unit: '%', step: 1 },
  rotation: { label: 'Rotation', rest: 0, unit: '°', step: 1 },
  skew: { label: 'Skew', rest: 0, unit: '°', step: 1 },
  tracking: { label: 'Tracking', rest: 0, unit: 'px', step: 0.5 },
  blur: { label: 'Blur', rest: 0, unit: 'px', min: 0, step: 0.5 },
  color: { label: 'Color', rest: '#ffffff' }
} as const;
export type TextAnimatorProperty = keyof typeof TEXT_ANIMATOR_PROPERTIES;

export const TEXT_ANIMATOR_SETTINGS: Record<string, string> = {
  amount: 'Amount', delay: 'Start', duration: 'Duration', stagger: 'Stagger',
  start: 'Range start', end: 'Range end', offset: 'Range offset', smoothness: 'Softness',
  speed: 'Speed', spread: 'Spread', unit: 'Based on'
};

export const textAnimatorLabel = (key: string): string =>
  (TEXT_ANIMATOR_PROPERTIES as Record<string, { label: string }>)[key]?.label ?? TEXT_ANIMATOR_SETTINGS[key] ?? key;

export interface TextAnimatorState {
  id: string;
  mode: TextAnimatorMode;
  unit: TextAnimatorUnit;
  /** Evaluated channel values at the requested time. */
  values: Record<string, any>;
  /** Selection weight per unit, in layout order. */
  weights: number[];
}

const num = (value: unknown, fallback = 0): number => {
  const n = Number(value);
  return Number.isFinite(n) ? n : fallback;
};
const clamp01 = (value: number): number => Math.max(0, Math.min(1, value));

export function animatorMode(animator: any): TextAnimatorMode {
  return animator?.mode === 'stagger' || animator?.mode === 'wave' ? animator.mode : 'range';
}

export function textControlValues(PM: any, layer: any, time: number) {
  return ['animators', 'styles'].map(kind => (layer.d[kind] || [])
    .filter((item: any) => kind !== 'animators' || item.enabled !== false)
    .map((item: any) => ({
      ...item,
      p: Object.fromEntries(Object.entries(item.p || {}).map(([k, p]) =>
        [k, PM.evP(layer, p, time, `${kind === 'animators' ? 'ta' : 'ts'}.${item.id}.${k}`)]))
    })));
}

/* Deterministic shuffle so "random" order is stable across frames and exports. */
const permutations = new Map<string, number[]>();
function randomRanks(count: number, seed: number): number[] {
  const key = count + ':' + seed;
  let ranks = permutations.get(key);
  if (ranks) return ranks;
  let s = (Math.floor(seed) >>> 0) || 1;
  const next = () => {
    s = (s + 0x6D2B79F5) >>> 0;
    let t = s;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
  const order = Array.from({ length: count }, (_, i) => i);
  for (let i = count - 1; i > 0; i--) {
    const j = Math.floor(next() * (i + 1));
    [order[i], order[j]] = [order[j]!, order[i]!];
  }
  ranks = new Array(count);
  order.forEach((unit, rank) => { ranks![unit] = rank; });
  permutations.set(key, ranks);
  if (permutations.size > 64) permutations.delete(permutations.keys().next().value!);
  return ranks;
}

/** Position of a unit in the animator's sequence. Center/edges use distance
 *  from the middle, so symmetric units share a rank and start together. */
export function unitRank(order: TextAnimatorOrder | undefined, index: number, count: number, seed = 1): number {
  const mid = (count - 1) / 2;
  switch (order) {
    case 'reverse': return count - 1 - index;
    case 'center': return Math.abs(index - mid);
    case 'edges': return mid - Math.abs(index - mid);
    case 'random': return randomRanks(count, seed)[index] ?? index;
    default: return index;
  }
}

function rankSpan(order: TextAnimatorOrder | undefined, count: number): number {
  return order === 'center' || order === 'edges' ? Math.max(0, (count - 1) / 2) : Math.max(0, count - 1);
}

/** Range selector weight. `rank` 0 is the first unit in the chosen order. */
export function animatorWeight(p: any, rank: number, count: number, shape: TextAnimatorShape = 'square'): number {
  const position = (rank + .5) / Math.max(1, count) * 100 - num(p.offset);
  const start = Math.min(num(p.start), num(p.end, 100)), end = Math.max(num(p.start), num(p.end, 100));
  const span = end - start;
  if (shape === 'rampUp' || shape === 'rampDown') {
    const u = span > 0 ? clamp01((position - start) / span) : position >= start ? 1 : 0;
    return shape === 'rampUp' ? u : 1 - u;
  }
  if (position < start || position > end) return 0;
  if (shape === 'triangle' || shape === 'round') {
    const u = span > 0 ? (position - start) / span : .5;
    return shape === 'triangle' ? 1 - Math.abs(u * 2 - 1) : Math.sin(Math.PI * u);
  }
  const feather = Math.max(0, num(p.smoothness)) / 100 * span / 2;
  if (!feather) return 1;
  const weight = clamp01(Math.min((position - start) / feather, (end - position) / feather));
  return weight * weight * (3 - 2 * weight);
}

const SPRING = { damping: 11, stiffness: 170, mass: 1 };
/** Eased progress for a stagger unit. Back and Spring overshoot past 1. */
export function easeProgress(PM: any, easing: string | undefined, progress: number): number {
  const t = clamp01(progress);
  if (t <= 0 || t >= 1) return t;
  const Ease = PM?.Ease;
  if (easing === 'spring') {
    if (!Ease?.spring) return 1 - Math.pow(1 - t, 3);
    const settle = Ease.springDuration(SPRING);
    return Ease.spring(t * settle, SPRING);
  }
  if (Ease?.PRESETS?.[easing || '']) return Ease.fn(easing)(t);
  if (easing === 'linear') return t;
  return 1 - Math.pow(1 - t, 3);
}

/** Seconds from the animator's start until every unit has settled. */
export function staggerLength(values: any, count: number, order?: TextAnimatorOrder): number {
  return Math.max(0, num(values.duration, .5)) + Math.max(0, num(values.stagger)) * rankSpan(order, count);
}

/** Unit counts for wrapped lines, segmented exactly as the text layout does. */
export function countTextUnits(lines: Array<{ text: string }>, unit: TextAnimatorUnit): number {
  if (unit === 'lines') return lines.filter(line => line.text.length).length;
  if (unit === 'words') return lines.reduce((sum, line) => sum + (line.text.match(/\S+/gu)?.length ?? 0), 0);
  if (lines.length === 0) return 0;
  const segmenter = typeof Intl !== 'undefined' && Intl.Segmenter
    ? new Intl.Segmenter(undefined, { granularity: 'grapheme' }) : null;
  let count = 0;
  lines.forEach(line => {
    if (segmenter) {
      for (const { segment } of segmenter.segment(line.text)) if (!/^\s+$/u.test(segment)) count++;
    } else {
      for (const segment of line.text) if (!/^\s+$/u.test(segment)) count++;
    }
  });
  return count;
}

/** The window a stagger animator occupies, in seconds from the layer's in point. */
export function staggerWindow(values: any, count: number, order?: TextAnimatorOrder) {
  const start = num(values.delay), duration = Math.max(0, num(values.duration, .5));
  const cascade = Math.max(0, num(values.stagger)) * rankSpan(order, count);
  return { start, duration, cascade, end: start + duration + cascade };
}

export function unitWeights(PM: any, layer: any, animator: any, values: any, count: number, time: number): number[] {
  const mode = animatorMode(animator);
  const offset = Math.max(0, Math.floor(num(animator.unitOffset)));
  const total = Math.max(count + offset, Math.floor(num(animator.unitTotal)) || 0, 1);
  const amount = num(values.amount, 100) / 100;
  const order: TextAnimatorOrder | undefined = animator.order;
  const local = time - num(layer.from);
  const weights = new Array(count);
  for (let i = 0; i < count; i++) {
    const rank = unitRank(order, offset + i, total, num(animator.seed, 1));
    let w: number;
    if (mode === 'stagger') {
      const duration = Math.max(0, num(values.duration, .5));
      const elapsed = local - num(values.delay) - rank * Math.max(0, num(values.stagger));
      const progress = duration > 0 ? elapsed / duration : elapsed >= 0 ? 1 : 0;
      const eased = easeProgress(PM, animator.easing, progress);
      w = animator.direction === 'out' ? eased : 1 - eased;
    } else if (mode === 'wave') {
      w = Math.sin(2 * Math.PI * (local * num(values.speed, 1) - rank * num(values.spread, 10) / 100));
    } else {
      const span = rankSpan(order, total), normalized = span > 0 ? rank / span * (total - 1) : rank;
      w = animatorWeight(values, normalized, total, animator.shape);
    }
    weights[i] = w * amount;
  }
  return weights;
}

function unitCount(layout: any, unit: TextAnimatorUnit): number {
  return layout[unit]?.length || (unit === 'characters' ? 0 : layout.characters.length ? 1 : 0);
}

/** Evaluate every enabled animator's values and per-unit weights. */
export function textAnimationState(PM: any, layer: any, time: number, layout: any): { animators: TextAnimatorState[]; styles: any[] } {
  const [animators, styles] = textControlValues(PM, layer, time);
  return {
    styles,
    animators: animators.map((animator: any) => {
      const values = animator.p;
      const unit: TextAnimatorUnit = ['characters', 'words', 'lines'].includes(animator.unit)
        ? animator.unit : ['words', 'lines'].includes(values.unit) ? values.unit : 'characters';
      return { id: animator.id, mode: animatorMode(animator), unit, values,
        weights: unitWeights(PM, layer, animator, values, unitCount(layout, unit), time) };
    })
  };
}

/** Stable cache key: identical keys render identical pixels. */
export function textAnimationKey(state: { animators: TextAnimatorState[]; styles: any[] }): string {
  return JSON.stringify([
    state.styles.map((style: any) => [style.start, style.end, style.p]),
    state.animators.map(animator => [animator.unit, Object.fromEntries(Object.entries(animator.values)
      .filter(([key]) => key in TEXT_ANIMATOR_PROPERTIES)), animator.weights.map(w => Math.round(w * 1e4))])
  ]);
}

function parseColor(value: unknown): [number, number, number] | null {
  const text = String(value || '').trim();
  let match = /^#([0-9a-f]{3,8})$/i.exec(text);
  if (match) {
    let hex = match[1]!;
    if (hex.length <= 4) hex = [...hex.slice(0, 3)].map(c => c + c).join('');
    return [parseInt(hex.slice(0, 2), 16), parseInt(hex.slice(2, 4), 16), parseInt(hex.slice(4, 6), 16)];
  }
  match = /^rgba?\(\s*([\d.]+)[\s,]+([\d.]+)[\s,]+([\d.]+)/i.exec(text);
  return match ? [num(match[1]), num(match[2]), num(match[3])] : null;
}
function mixColor(from: string, to: string, weight: number): string {
  const w = clamp01(weight);
  if (w <= 0) return from;
  const a = parseColor(from), b = parseColor(to);
  if (!a || !b) return w >= .5 ? to : from;
  const channel = (i: number) => Math.round(a[i]! + (b[i]! - a[i]!) * w).toString(16).padStart(2, '0');
  return '#' + channel(0) + channel(1) + channel(2);
}

type Matrix = [number, number, number, number, number, number];
const multiply = (m: Matrix, n: Matrix): Matrix => [
  m[0] * n[0] + m[2] * n[1], m[1] * n[0] + m[3] * n[1],
  m[0] * n[2] + m[2] * n[3], m[1] * n[2] + m[3] * n[3],
  m[0] * n[4] + m[2] * n[5] + m[4], m[1] * n[4] + m[3] * n[5] + m[5]
];

export function isIdentityGlyph(glyph: any): boolean {
  const m = glyph.matrix;
  return m[0] === 1 && m[1] === 0 && m[2] === 0 && m[3] === 1 && m[4] === 0 && m[5] === 0
    && glyph.opacity === 1 && !glyph.blur && !glyph.color && !glyph.trackingShift;
}

/**
 * Glyphs with their animated pose. `matrix` is relative to the glyph's
 * baseline origin (x, y + baseline); scale, rotation and skew pivot around the
 * center of the glyph's unit, so words and lines transform as a whole.
 */
export function animatedGlyphs(PM: any, layer: any, time: number, d: any, layout: any,
  state = textAnimationState(PM, layer, time, layout)) {
  const size = Math.max(1, num(d.size, 16)), pivotY = -size * .35;
  const glyphs = layout.characters.map((glyph: any) => {
    let style = { ...d };
    for (const range of state.styles) if (glyph.sourceStart >= range.start && glyph.sourceStart < range.end) style = { ...style, ...range.p };
    let matrix: Matrix = [1, 0, 0, 1, 0, 0], opacity = 1, blur = 0, tracking = 0, color: string | null = null;
    for (const animator of state.animators) {
      const index = animator.unit === 'lines' ? glyph.lineUnit ?? glyph.line : animator.unit === 'words' ? glyph.word : glyph.index;
      const w = animator.weights[index] ?? 0;
      if (!w) continue;
      const p = animator.values;
      const unit = animator.unit === 'characters' ? glyph : layout[animator.unit]?.[index];
      const px = unit ? num(unit.x) + num(unit.w) / 2 - num(glyph.x) : num(glyph.w) / 2;
      const r = num(p.rotation) * w * Math.PI / 180, k = Math.tan(Math.max(-80, Math.min(80, num(p.skew) * w)) * Math.PI / 180);
      const s = 1 + (num(p.scale, 100) / 100 - 1) * w;
      const cos = Math.cos(r) * s, sin = Math.sin(r) * s;
      // T(pivot + offset) · R · S · Skew · T(−pivot)
      const local: Matrix = [cos, sin, cos * -k - sin, sin * -k + cos, 0, 0];
      local[4] = px + num(p.x) * w - (local[0] * px + local[2] * pivotY);
      local[5] = pivotY + num(p.y) * w - (local[1] * px + local[3] * pivotY);
      matrix = multiply(local, matrix);
      opacity *= 1 + (num(p.opacity, 100) / 100 - 1) * w;
      blur += Math.max(0, num(p.blur)) * Math.abs(w);
      tracking += num(p.tracking) * w;
      if (p.color != null && p.color !== '') color = mixColor(color ?? String(style.color), String(p.color), w);
    }
    return { ...glyph, style, matrix, opacity: clamp01(opacity), blur, color, tracking };
  });
  // Tracking widens the gap after each unit, pushing the rest of the line.
  const align = d.align === 'center' ? .5 : d.align === 'right' ? 1 : 0;
  const runs = new Map<any, any[]>();
  glyphs.forEach((glyph: any) => {
    // The old strict-equality scan ignored NaN line identifiers.
    if (glyph.line !== glyph.line) return;
    const run = runs.get(glyph.line);
    if (run) run.push(glyph); else runs.set(glyph.line, [glyph]);
  });
  for (const run of runs.values()) {
    let shift = 0;
    for (const glyph of run) { glyph.trackingShift = shift; shift += glyph.tracking; }
    const lead = shift - (run.at(-1)?.tracking ?? 0);
    for (const glyph of run) { glyph.trackingShift -= lead * align; glyph.x += glyph.trackingShift; }
  }
  return glyphs;
}
