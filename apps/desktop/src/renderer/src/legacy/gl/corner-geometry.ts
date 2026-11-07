/* Rounded-rectangle corner model shared by every shape renderer: independent
   per-corner radii and Figma-style corner smoothing (a continuous-curvature
   "squircle" blend between each edge and its arc). Content keeps `radius` as the
   uniform value; `independentCorners` switches to radiusTL/TR/BR/BL, and
   `smoothing` is a percentage (0 = circular arc, 100 = fully smoothed). */

export type CornerRadii = [topLeft: number, topRight: number, bottomRight: number, bottomLeft: number];

export const CORNER_KEYS = ['radiusTL', 'radiusTR', 'radiusBR', 'radiusBL'] as const;
/** The iOS app-icon smoothing preset. */
export const IOS_SMOOTHING = 60;

const number = (value: unknown): number => {
  const result = Number(value);
  return Number.isFinite(result) ? Math.max(0, result) : 0;
};

export function cornerRadii(d: any): CornerRadii {
  const uniform = number(d.radius);
  if (!d.independentCorners) return [uniform, uniform, uniform, uniform];
  return CORNER_KEYS.map(key => (d[key] == null ? uniform : number(d[key]))) as CornerRadii;
}

/** Smoothing as a 0..1 fraction. */
export function cornerSmoothing(d: any): number {
  const percent = Number(d.smoothing);
  return Number.isFinite(percent) ? Math.min(1, Math.max(0, percent / 100)) : 0;
}

export function isUniformCircular(d: any): boolean {
  const [a, b, c, e] = cornerRadii(d);
  return cornerSmoothing(d) === 0 && a === b && b === c && c === e;
}

/** Clamp radii like Figma: each is limited to half the shorter side. */
function clampedRadii(d: any): CornerRadii {
  const limit = Math.max(0, Math.min(Number(d.w) || 0, Number(d.h) || 0) / 2);
  return cornerRadii(d).map(r => Math.min(r, limit)) as CornerRadii;
}

/** Distance from the bounding box inside which corner geometry can differ from a flat fill. */
export function cornerInset(d: any): number {
  const smoothing = cornerSmoothing(d);
  const reach = Math.max(...clampedRadii(d)) * (1 + smoothing);
  return Math.max(0, Math.min(reach, (Number(d.w) || 0) / 2, (Number(d.h) || 0) / 2));
}

interface CornerParams { r: number; a: number; b: number; c: number; d: number; p: number; arc: number }

const rad = (degrees: number): number => degrees * Math.PI / 180;

function cornerParams(r: number, smoothing: number, budget: number): CornerParams {
  if (r <= 0) return { r: 0, a: 0, b: 0, c: 0, d: 0, p: 0, arc: 0 };
  // Preserve smoothing when adjacent corners fill an edge (including pills).
  // Shrink the circular core instead of reducing smoothing to zero. Both
  // branches meet continuously at r * (1 + smoothing) === budget.
  const s = smoothing;
  r = Math.min(r, budget / (1 + s));
  const p = Math.min((1 + s) * r, budget);
  const arcMeasure = 90 * (1 - s);
  const arc = Math.sin(rad(arcMeasure / 2)) * r * Math.SQRT2;
  const alpha = (90 - arcMeasure) / 2;
  const p3p4 = r * Math.tan(rad(alpha / 2));
  const beta = 45 * s;
  const c = p3p4 * Math.cos(rad(beta));
  const d = c * Math.tan(rad(beta));
  // Match the cubic's end curvature (2*b*d / (3*|tangent|^3))
  // to the circular arc's 1/r, as well as matching their tangents.
  const b = d > 0 ? 1.5 * Math.hypot(c, d) ** 3 / (d * r) : 0;
  const a = p - arc - c - d - b;
  return { r, a, b, c, d, p, arc };
}

/** Room each corner may use along its edges, shared proportionally with its neighbours. */
function budgets(radii: CornerRadii, w: number, h: number): number[] {
  const [tl, tr, br, bl] = radii;
  const share = (r: number, neighbour: number, side: number) => (r + neighbour === 0 ? side : r / (r + neighbour) * side);
  return [
    Math.min(share(tl, tr, w), share(tl, bl, h)),
    Math.min(share(tr, tl, w), share(tr, br, h)),
    Math.min(share(br, bl, w), share(br, tr, h)),
    Math.min(share(bl, br, w), share(bl, tl, h))
  ];
}

const f = (value: number): string => String(Math.round(value * 1e4) / 1e4);

/** SVG path data for the shape's outline with its origin at the top-left corner. */
export function roundedRectPath(d: any): string {
  const w = Number(d.w) || 0, h = Number(d.h) || 0;
  const radii = clampedRadii(d);
  const smoothing = cornerSmoothing(d);
  const budget = budgets(radii, w, h);
  const [tl, tr, br, bl] = radii.map((r, i) => cornerParams(r, smoothing, budget[i]!)) as [CornerParams, CornerParams, CornerParams, CornerParams];
  const arc = (k: CornerParams, x: number, y: number) => `a ${f(k.r)} ${f(k.r)} 0 0 1 ${f(x)} ${f(y)}`;
  const curve = (...values: number[]) => `c ${values.map(f).join(' ')}`;
  const segments = [`M ${f(w - tr.p)} 0`];
  segments.push(tr.r
    ? [curve(tr.a, 0, tr.a + tr.b, 0, tr.a + tr.b + tr.c, tr.d), arc(tr, tr.arc, tr.arc),
      curve(tr.d, tr.c, tr.d, tr.b + tr.c, tr.d, tr.a + tr.b + tr.c)].join(' ')
    : `l ${f(tr.p)} 0`);
  segments.push(`L ${f(w)} ${f(h - br.p)}`);
  segments.push(br.r
    ? [curve(0, br.a, 0, br.a + br.b, -br.d, br.a + br.b + br.c), arc(br, -br.arc, br.arc),
      curve(-br.c, br.d, -(br.b + br.c), br.d, -(br.a + br.b + br.c), br.d)].join(' ')
    : `l 0 ${f(br.p)}`);
  segments.push(`L ${f(bl.p)} ${f(h)}`);
  segments.push(bl.r
    ? [curve(-bl.a, 0, -(bl.a + bl.b), 0, -(bl.a + bl.b + bl.c), -bl.d), arc(bl, -bl.arc, -bl.arc),
      curve(-bl.d, -bl.c, -bl.d, -(bl.b + bl.c), -bl.d, -(bl.a + bl.b + bl.c))].join(' ')
    : `l ${f(-bl.p)} 0`);
  segments.push(`L 0 ${f(tl.p)}`);
  segments.push(tl.r
    ? [curve(0, -tl.a, 0, -(tl.a + tl.b), tl.d, -(tl.a + tl.b + tl.c)), arc(tl, tl.arc, -tl.arc),
      curve(tl.c, -tl.d, tl.b + tl.c, -tl.d, tl.a + tl.b + tl.c, -tl.d)].join(' ')
    : `l 0 ${f(-tl.p)}`);
  segments.push('Z');
  return segments.join(' ');
}
