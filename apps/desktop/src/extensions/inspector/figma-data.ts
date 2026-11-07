/*
 * Inspector helpers for the editable drawing data a Figma import produces
 * (docs/figma-layer-data.md): gradients, stroke style, arcs, image mapping
 * and styled text runs. Values live under nested content paths such as
 * `c.fillGradient.stops.1.color`; scalar leaves become `{v,kf,expr}` channels
 * on first edit, so ordinary edits go through `set_property`. Adding or
 * removing array members (stops, dashes) or the whole paint is structural and
 * goes through `set_content` with the raw (channel-preserving) object.
 */
import type { EditCommand } from 'powermove';

export type Matrix = [number, number, number, number, number, number];
export const IDENTITY: Matrix = [1, 0, 0, 1, 0, 0];
export const GRADIENT_TYPES = ['linear', 'radial', 'angular', 'diamond'] as const;
export type GradientType = (typeof GRADIENT_TYPES)[number];
export const GRADIENT_LABELS: Record<GradientType, string> = { linear: 'Linear', radial: 'Radial', angular: 'Angular', diamond: 'Diamond' };

const isChannel = (value: unknown): value is { v: unknown; kf: unknown[] } =>
  !!value && typeof value === 'object' && Array.isArray((value as { kf?: unknown }).kf) && 'v' in (value as object);

/** The stored (possibly channel) value at a dotted content key, e.g. `arc.start`. */
export function rawAt(layer: unknown, key: string): unknown {
  let owner: unknown = (layer as { d?: unknown } | null | undefined)?.d;
  // Flat keys may contain dots themselves (`fontAxis.wght`).
  if (owner && typeof owner === 'object' && Object.hasOwn(owner, key)) return (owner as Record<string, unknown>)[key];
  for (const part of key.split('.')) {
    if (!owner || typeof owner !== 'object' || !Object.hasOwn(owner as object, part)) return undefined;
    owner = (owner as Record<string, unknown>)[part];
  }
  return owner;
}

/** A plain, static value for a raw leaf; channels report their static value. */
export const staticValue = (value: unknown): unknown => isChannel(value) ? value.v : value;

export function multiply(a: Matrix, b: Matrix): Matrix {
  return [
    a[0] * b[0] + a[2] * b[1], a[1] * b[0] + a[3] * b[1],
    a[0] * b[2] + a[2] * b[3], a[1] * b[2] + a[3] * b[3],
    a[0] * b[4] + a[2] * b[5] + a[4], a[1] * b[4] + a[3] * b[5] + a[5]
  ];
}

export function invert(m: Matrix): Matrix {
  const det = m[0] * m[3] - m[1] * m[2];
  if (!Number.isFinite(det) || Math.abs(det) < 1e-12) return [...IDENTITY];
  const a = m[3] / det, b = -m[1] / det, c = -m[2] / det, d = m[0] / det;
  return [a, b, c, d, -(a * m[4] + c * m[5]), -(b * m[4] + d * m[5])];
}

const toMatrix = (value: unknown): Matrix => {
  const list = Array.isArray(value) ? value.map((item) => Number(staticValue(item))) : [];
  return list.length === 6 && list.every(Number.isFinite) ? list as Matrix : [...IDENTITY];
};

/**
 * The paint's axis in layer pixels. `transform` maps normalized object
 * coordinates to paint coordinates, where a linear gradient runs from
 * (0, .5) to (1, .5). Its inverse places that axis on the layer.
 */
function axes(transform: unknown, w: number, h: number) {
  const m = invert(toMatrix(transform));
  const a = { x: m[0] * w, y: m[1] * h }, b = { x: m[2] * w, y: m[3] * h };
  const center = { x: (m[0] * 0.5 + m[2] * 0.5 + m[4]) * w, y: (m[1] * 0.5 + m[3] * 0.5 + m[5]) * h };
  return { a, b, center };
}

/** Length that reads as 100%: a linear gradient spans the box along its angle (as in CSS). */
const reference = (type: string, radians: number, w: number, h: number) =>
  type === 'linear' ? Math.max(1e-6, Math.abs(w * Math.cos(radians)) + Math.abs(h * Math.sin(radians))) : Math.max(1e-6, w);

const round = (value: number, digits = 4) => Math.round(value * 10 ** digits) / 10 ** digits + 0;

/** Angle in degrees (0 = left to right, clockwise on screen) and scale (1 = fits the box). */
export function gradientGeometry(transform: unknown, type: string, w: number, h: number): { angle: number; scale: number } {
  const width = Math.max(1, Number(w) || 1), height = Math.max(1, Number(h) || 1);
  const { a } = axes(transform, width, height);
  const radians = Math.atan2(a.y, a.x);
  return {
    angle: round((radians * 180) / Math.PI, 2),
    scale: round(Math.hypot(a.x, a.y) / reference(type, radians, width, height), 4)
  };
}

/** A new transform with this angle/scale around the paint's current center; skew and aspect are kept. */
export function composeGradient(transform: unknown, type: string, w: number, h: number, angle: number, scale: number): Matrix {
  const width = Math.max(1, Number(w) || 1), height = Math.max(1, Number(h) || 1);
  const { a, b, center } = axes(transform, width, height);
  const before = Math.atan2(a.y, a.x), radians = (angle * Math.PI) / 180;
  const length = Math.max(1e-6, Math.abs(scale)) * reference(type, radians, width, height);
  const ratio = length / Math.max(1e-9, Math.hypot(a.x, a.y));
  const turn = radians - before, cos = Math.cos(turn), sin = Math.sin(turn);
  const a2 = { x: Math.cos(radians) * length, y: Math.sin(radians) * length };
  const b2 = { x: (b.x * cos - b.y * sin) * ratio, y: (b.x * sin + b.y * cos) * ratio };
  const origin = { x: center.x - (a2.x + b2.x) / 2, y: center.y - (a2.y + b2.y) / 2 };
  const inverseTransform: Matrix = [a2.x / width, a2.y / height, b2.x / width, b2.y / height, origin.x / width, origin.y / height];
  return invert(inverseTransform).map((value) => round(value, 6)) as Matrix;
}

export interface Rgba { r: number; g: number; b: number; a: number }
export function parseHex(value: unknown): Rgba {
  const hex = String(value ?? '').replace('#', '');
  const full = hex.length === 3 || hex.length === 4 ? [...hex].map((c) => c + c).join('') : hex;
  const read = (i: number, fallback: number) => {
    const n = parseInt(full.slice(i, i + 2), 16);
    return Number.isFinite(n) ? n : fallback;
  };
  return { r: read(0, 0), g: read(2, 0), b: read(4, 0), a: full.length >= 8 ? read(6, 255) : 255 };
}
export const toHex = ({ r, g, b, a }: Rgba) =>
  '#' + [r, g, b, a].map((n) => Math.round(Math.max(0, Math.min(255, n))).toString(16).padStart(2, '0')).join('').toUpperCase();

/** The color a gradient shows at `position`, so an inserted stop changes nothing until edited. */
export function colorAt(stops: Array<{ position: number; color: string }>, position: number): string {
  const sorted = [...stops].sort((x, y) => x.position - y.position);
  if (!sorted.length) return '#000000FF';
  if (position <= sorted[0]!.position) return toHex(parseHex(sorted[0]!.color));
  for (let i = 1; i < sorted.length; i++) {
    const lo = sorted[i - 1]!, hi = sorted[i]!;
    if (position <= hi.position) {
      const t = hi.position - lo.position > 1e-9 ? (position - lo.position) / (hi.position - lo.position) : 0;
      const p = parseHex(lo.color), q = parseHex(hi.color);
      return toHex({ r: p.r + (q.r - p.r) * t, g: p.g + (q.g - p.g) * t, b: p.b + (q.b - p.b) * t, a: p.a + (q.a - p.a) * t });
    }
  }
  return toHex(parseHex(sorted.at(-1)!.color));
}

/** Where a new stop goes: halfway into the widest gap next to the selected stop. */
export function nextStopPosition(stops: Array<{ position: number }>, selected: number): number {
  const sorted = stops.map((stop) => stop.position).sort((x, y) => x - y);
  const at = stops[selected]?.position ?? sorted[0] ?? 0;
  const index = sorted.indexOf(at);
  const after = sorted[index + 1], before = sorted[index - 1];
  const right = after === undefined ? -1 : after - at, left = before === undefined ? -1 : at - before;
  if (right <= 0 && left <= 0) return at < 0.5 ? Math.min(1, at + 0.5) : Math.max(0, at - 0.5);
  return round(right >= left ? at + right / 2 : at - left / 2, 4);
}

/** A two-stop gradient that starts from the current solid color and fades it out. */
export function defaultGradient(color: unknown, type: GradientType = 'linear') {
  const solid = parseHex(color);
  const start = toHex({ ...solid, a: solid.a || 255 });
  return {
    type,
    transform: [...IDENTITY],
    stops: [{ position: 0, color: start }, { position: 1, color: toHex({ ...solid, a: 0 }) }],
    opacity: 1
  };
}

/** Deep copy of raw content data, keeping channel records intact. */
export const cloneRaw = <T>(value: T): T => (value === undefined ? value : structuredClone(value));

export type TimeSource = () => number;

/** `set_property` on a nested content path, keyed at the playhead when the channel is animated. */
export function propertyCommand(target: string, path: string, value: unknown, time: number): EditCommand {
  return {
    type: 'set_property',
    target,
    path,
    value,
    time,
    mode: 'auto',
    preserveHandEdits: false,
    markIntent: 'human'
  } as unknown as EditCommand;
}

export function contentCommand(target: string, patch: Record<string, unknown>): EditCommand {
  return { type: 'set_content', target, patch } as unknown as EditCommand;
}

export const STROKE_POSITIONS = [
  { v: 'center', label: 'Center' },
  { v: 'inside', label: 'Inside' },
  { v: 'outside', label: 'Outside' }
] as const;
export const JOINS = [
  { v: 'miter', label: 'Miter' },
  { v: 'round', label: 'Round' },
  { v: 'bevel', label: 'Bevel' }
] as const;
export const DASH_CAPS = [
  { v: 'NONE', label: 'None' },
  { v: 'ROUND', label: 'Round' },
  { v: 'SQUARE', label: 'Square' }
];
/** Endpoint caps for open paths, in Figma's names. */
export const END_CAPS = [
  { v: 'NONE', label: 'None' },
  { v: 'ROUND', label: 'Round' },
  { v: 'SQUARE', label: 'Square' },
  { v: 'ARROW_LINES', label: 'Line Arrow' },
  { v: 'ARROW_EQUILATERAL', label: 'Triangle Arrow' },
  { v: 'TRIANGLE_FILLED', label: 'Triangle' },
  { v: 'CIRCLE_FILLED', label: 'Circle' },
  { v: 'DIAMOND_FILLED', label: 'Diamond' }
];
export const IMAGE_MODES = [
  { v: 'fill', label: 'Fill' },
  { v: 'fit', label: 'Fit' },
  { v: 'crop', label: 'Crop' },
  { v: 'tile', label: 'Tile' }
] as const;
export const VERTICAL_ALIGN = [
  { v: 'top', label: 'Top' },
  { v: 'center', label: 'Middle' },
  { v: 'bottom', label: 'Bottom' }
] as const;
export const DECORATIONS = [
  { v: 'NONE', label: 'None' },
  { v: 'UNDERLINE', label: 'Underline' },
  { v: 'STRIKETHROUGH', label: 'Strikethrough' }
];
export const TEXT_CASES = [
  { v: 'ORIGINAL', label: 'As Typed' },
  { v: 'UPPER', label: 'Uppercase' },
  { v: 'LOWER', label: 'Lowercase' },
  { v: 'TITLE', label: 'Title Case' },
  { v: 'SMALL_CAPS', label: 'Small Caps' },
  { v: 'SMALL_CAPS_FORCED', label: 'All Small Caps' }
];
export const WEIGHT_NAMES: Record<number, string> = {
  100: 'Thin', 200: 'Extra Light', 300: 'Light', 400: 'Regular', 500: 'Medium',
  600: 'Semi Bold', 700: 'Bold', 800: 'Extra Bold', 900: 'Black'
};
export const styleName = (weight: number, italic: boolean) => {
  const name = WEIGHT_NAMES[Math.round(weight / 100) * 100] ?? String(weight);
  return italic ? (name === 'Regular' ? 'Italic' : `${name} Italic`) : name;
};

/** Run fields that override the layer's base text style. */
export const RUN_FIELDS = ['font', 'size', 'weight', 'italic', 'color', 'tracking', 'leading', 'decoration', 'textCase', 'paragraphSpacing', 'paragraphIndent'] as const;
