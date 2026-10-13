/**
 * Writes tests/vectors/geometry-vectors.json by running today's geometry code
 * under bun against a recording Canvas2D context. pm-vector must issue the
 * same canvas calls and compute the same bounds, bitmap sizes, and mask
 * uniforms, bit for bit.
 *
 *   bun engine/crates/pm-vector/tests/vectors/generate-geometry-vectors.ts
 *
 * Sources exercised:
 * - legacy/gl/corner-geometry.ts (imported): cornerRadii, cornerSmoothing,
 *   cornerInset, isUniformCircular, roundedRectPath.
 * - legacy/gl/shape-raster-window.ts (imported): shapeRasterGeometry.
 * - legacy/gl/raster.ts: getCanvas, rr, and rasterShape. raster.ts pulls in
 *   three.js and the DOM at import time and does not export rasterShape, so
 *   the generator slices those functions out of the file's current source,
 *   transpiles them with Bun.Transpiler, and runs them with a fake canvas.
 * - legacy/core/vector-paths.ts (imported): pathValues, pathPoints,
 *   tracePath, groupMatrix, rasterPaths. `PM.mul` and `PM.clamp` are sliced
 *   from legacy/core/anim.ts and legacy/core/util.ts the same way.
 * - legacy/gl/compositor.ts: applyMasks (the parametric GPU route; its
 *   uniforms are captured through a fake `setU`/`setI`) and applyPathMasks
 *   (the path route; its coverage canvases are recorded), sliced the same way.
 *   FRAG_MASK itself is GLSL; `coverage` below is a line-by-line JavaScript
 *   transliteration of it in double precision, so that comparison has a
 *   tolerance.
 *
 * The recording context implements the HTML specification's rules the legacy
 * code relies on: transform calls with non-finite arguments are ignored,
 * `arcTo` and `ellipse` throw IndexSizeError for negative radii (after the
 * non-finite check), and the `lineWidth` and `globalAlpha` setters ignore
 * invalid values. Path calls are recorded verbatim; the Rust test replays them
 * through pm-vector's CanvasPath, which implements the remaining rules.
 *
 * Numbers that JSON cannot carry are written as the strings "NaN",
 * "Infinity", "-Infinity", and "-0". Deterministic: a seeded PRNG.
 */
import { readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import * as corner from '../../../../../apps/desktop/src/renderer/src/legacy/gl/corner-geometry';
import { shapeRasterGeometry } from '../../../../../apps/desktop/src/renderer/src/legacy/gl/shape-raster-window';
import { groupMatrix, pathPoints, pathValues, rasterPaths, tracePath } from '../../../../../apps/desktop/src/renderer/src/legacy/core/vector-paths';
import { isProperty } from '../../../../../apps/desktop/src/renderer/src/legacy/core/content-properties';

const legacy = path.resolve(import.meta.dir, '../../../../../apps/desktop/src/renderer/src/legacy');

/* ── V8's Math.hypot ──
 * The editor runs on V8. JavaScriptCore's Math.hypot (bun) rounds differently,
 * which moves trim positions and mask blur radii by an ulp, so the generator
 * installs V8's algorithm (src/builtins/math.tq MathHypot: scale by the
 * largest magnitude, Kahan-compensated sum of squares) and checks it against
 * node's V8 bit for bit when node is installed. */
const jscHypot = Math.hypot;
function v8Hypot(...args: number[]): number {
  let max = 0, nan = false;
  const abs = args.map(a => { const v = Number(a); if (Number.isNaN(v)) { nan = true; return 0; } const x = Math.abs(v); if (x > max) max = x; return x; });
  if (max === Infinity) return Infinity;
  if (nan) return NaN;
  if (max === 0) return 0;
  let sum = 0, compensation = 0;
  for (const value of abs) {
    const n = value / max, summand = n * n - compensation, preliminary = sum + summand;
    compensation = (preliminary - sum) - summand;
    sum = preliminary;
  }
  return Math.sqrt(sum) * max;
}
Math.hypot = v8Hypot;

/* ── sources sliced from modules that cannot be imported headlessly ── */
const transpiler = new Bun.Transpiler({ loader: 'ts' });
function slice(file: string, from: string, to: string): string {
  const source = readFileSync(path.join(legacy, file), 'utf8');
  const a = source.indexOf(from);
  const b = a < 0 ? -1 : source.indexOf(to, a);
  if (a < 0 || b < 0) throw new Error(`${file}: cannot find ${JSON.stringify(from)} .. ${JSON.stringify(to)}`);
  return source.slice(a, b);
}
/** Transpiles `source` and returns the named bindings, given its free variables. */
function load(source: string, names: string[], free: Record<string, unknown>): Record<string, any> {
  const js = transpiler.transformSync(`${source}\nglobalThis.__pmVectorExports = { ${names.join(', ')} };`);
  new Function(...Object.keys(free), js)(...Object.values(free));
  const out = (globalThis as any).__pmVectorExports;
  delete (globalThis as any).__pmVectorExports;
  return out;
}

const { mul } = load(slice('core/anim.ts', 'function mul(', 'PM.mul = mul;'), ['mul'], {});
const { clamp } = load(slice('core/util.ts', 'const clamp =', '\nconst lerp'), ['clamp'], {});
const PM = { mul, clamp, evP: (_layer: unknown, p: any) => p };

/* ── recording Canvas2D ── */
type Matrix = [number, number, number, number, number, number];
const finite = (...values: number[]) => values.every(Number.isFinite);
const multiply = (m: Matrix, n: Matrix): Matrix => [
  m[0] * n[0] + m[2] * n[1], m[1] * n[0] + m[3] * n[1],
  m[0] * n[2] + m[2] * n[3], m[1] * n[2] + m[3] * n[3],
  m[0] * n[4] + m[2] * n[5] + m[4], m[1] * n[4] + m[3] * n[5] + m[5],
];
class IndexSizeError extends Error {}
class FakePath2D { constructor(public d: string) {} }
type State = { ctm: Matrix; lineWidth: number; lineCap: string; lineJoin: string; globalAlpha: number;
  fillStyle: string; strokeStyle: string; filter: string; globalCompositeOperation: string };

class Recorder {
  ops: any[] = [];
  events: any[] = [];
  s: State = { ctm: [1, 0, 0, 1, 0, 0], lineWidth: 1, lineCap: 'butt', lineJoin: 'miter', globalAlpha: 1,
    fillStyle: '#000000', strokeStyle: '#000000', filter: 'none', globalCompositeOperation: 'source-over' };
  stack: State[] = [];
  get lineWidth() { return this.s.lineWidth; }
  set lineWidth(v: number) { if (Number.isFinite(v) && v > 0) this.s.lineWidth = v; }
  get globalAlpha() { return this.s.globalAlpha; }
  set globalAlpha(v: number) { if (Number.isFinite(v) && v >= 0 && v <= 1) this.s.globalAlpha = v; }
  get lineCap() { return this.s.lineCap; }
  set lineCap(v: string) { if (['butt', 'round', 'square'].includes(v)) this.s.lineCap = v; }
  get lineJoin() { return this.s.lineJoin; }
  set lineJoin(v: string) { if (['round', 'bevel', 'miter'].includes(v)) this.s.lineJoin = v; }
  get fillStyle() { return this.s.fillStyle; }
  set fillStyle(v: string) { this.s.fillStyle = v; }
  get strokeStyle() { return this.s.strokeStyle; }
  set strokeStyle(v: string) { this.s.strokeStyle = v; }
  get filter() { return this.s.filter; }
  set filter(v: string) { this.s.filter = v; }
  get globalCompositeOperation() { return this.s.globalCompositeOperation; }
  set globalCompositeOperation(v: string) { this.s.globalCompositeOperation = v; }
  save() { this.stack.push({ ...this.s, ctm: [...this.s.ctm] as Matrix }); }
  restore() { const s = this.stack.pop(); if (s) this.s = s; }
  transform(...m: Matrix) { if (finite(...m)) this.s.ctm = multiply(this.s.ctm, m); }
  translate(x: number, y: number) { if (finite(x, y)) this.s.ctm = multiply(this.s.ctm, [1, 0, 0, 1, x, y]); }
  rotate(a: number) { if (finite(a)) { const c = Math.cos(a), s = Math.sin(a); this.s.ctm = multiply(this.s.ctm, [c, s, -s, c, 0, 0]); } }
  scale(x: number, y: number) { if (finite(x, y)) this.s.ctm = multiply(this.s.ctm, [x, 0, 0, y, 0, 0]); }
  beginPath() { this.ops = []; }
  moveTo(x: number, y: number) { this.ops.push(['M', x, y]); }
  lineTo(x: number, y: number) { this.ops.push(['L', x, y]); }
  bezierCurveTo(...a: number[]) { this.ops.push(['C', ...a]); }
  closePath() { this.ops.push(['Z']); }
  rect(x: number, y: number, w: number, h: number) { this.ops.push(['R', x, y, w, h]); }
  arcTo(x1: number, y1: number, x2: number, y2: number, r: number) {
    if (finite(x1, y1, x2, y2, r) && r < 0) throw new IndexSizeError('arcTo');
    this.ops.push(['A', x1, y1, x2, y2, r]);
  }
  ellipse(x: number, y: number, rx: number, ry: number, rotation: number, start: number, end: number, ccw?: boolean) {
    if (ccw) throw new Error('anticlockwise ellipses are not used by the legacy geometry');
    if (finite(x, y, rx, ry, rotation, start, end) && (rx < 0 || ry < 0)) throw new IndexSizeError('ellipse');
    this.ops.push(['E', x, y, rx, ry, rotation, start, end]);
  }
  private paint(op: string, p?: FakePath2D) {
    const s = this.s;
    this.events.push({ op, ctm: [...s.ctm], path: p ? { svg: p.d } : { ops: this.ops.map(o => [...o]) },
      alpha: s.globalAlpha, ...(op === 'stroke' ? { lineWidth: s.lineWidth, lineCap: s.lineCap, lineJoin: s.lineJoin, style: s.strokeStyle } : { style: s.fillStyle }) });
  }
  fill(p?: FakePath2D) { this.paint('fill', p); }
  stroke(p?: FakePath2D) { this.paint('stroke', p); }
  fillRect(x: number, y: number, w: number, h: number) { this.events.push({ op: 'fillRect', rect: [x, y, w, h], ctm: [...this.s.ctm] }); }
  drawImage(image: FakeCanvas) { this.events.push({ op: 'drawImage', canvas: image.id, filter: this.s.filter, composite: this.s.globalCompositeOperation }); }
}
let canvases: FakeCanvas[] = [];
class FakeCanvas {
  width = 300; height = 150; id = canvases.length; context = new Recorder();
  constructor() { canvases.push(this); }
  getContext() { return this.context; }
}
const fakeDocument = { createElement: (tag: string) => { if (tag !== 'canvas') throw new Error(tag); return new FakeCanvas(); } };
(globalThis as any).document = fakeDocument;

const { rasterShape } = load(
  slice('gl/raster.ts', 'function getCanvas(', '\nfunction release(') + slice('gl/raster.ts', 'function rr(', '/* ── captions'),
  ['rasterShape'],
  { window: { document: fakeDocument }, shapeRasterGeometry, isUniformCircular: corner.isUniformCircular, roundedRectPath: corner.roundedRectPath, Path2D: FakePath2D },
);

/* ── PRNG ── */
let seed = 0x6d2b79f5;
const random = () => { seed ^= seed << 13; seed >>>= 0; seed ^= seed >>> 17; seed ^= seed << 5; seed >>>= 0; return seed / 2 ** 32; };
const range = (lo: number, hi: number) => lo + random() * (hi - lo);
const int = (lo: number, hi: number) => lo + Math.floor(random() * (hi - lo + 1));
const chance = (p: number) => random() < p;
const pick = <T>(values: readonly T[]): T => values[Math.floor(random() * values.length)]!;
/** A number from `make`, occasionally replaced by an edge value. */
const edgy = (make: () => number, edges: readonly number[], p = 0.08) => (chance(p) ? pick(edges) : make());
const ODD = [0, -0, NaN, Infinity, -Infinity, -1, 1e-9, 1e9];

/* ── shapes and corners ── */
function shapeContent(): any {
  const d: any = { shape: pick(['rect', 'rect', 'rect', 'ellipse', 'polygon', 'star', 'line', 'triangle']), color: '#336699' };
  d.w = edgy(() => range(0, 1200), [0, -40, NaN, 0.5, 9000]);
  d.h = edgy(() => range(0, 1200), [0, -40, NaN, 0.5, 9000]);
  if (!chance(0.1)) d.radius = edgy(() => (chance(0.4) ? 0 : range(0, 400)), [-5, NaN, Infinity, 1e6]);
  d.independentCorners = chance(0.4);
  for (const key of corner.CORNER_KEYS) {
    if (chance(0.2)) continue;
    d[key] = chance(0.1) ? null : edgy(() => range(0, 300), [-10, NaN, Infinity, 0]);
  }
  if (!chance(0.15)) d.smoothing = chance(0.4) ? 0 : edgy(() => range(0, 120), [60, 100, -20, NaN, Infinity]);
  if (!chance(0.15)) d.stroke = chance(0.3) ? 0 : edgy(() => range(0, 30), [-3, NaN, Infinity, 0.25]);
  if (!chance(0.2)) d.points = edgy(() => (chance(0.5) ? int(3, 12) : range(0, 14)), [0, 1, 2, NaN, 64]);
  d.strokeColor = pick(['', '#ff8800']);
  return d;
}

function shapeCase(d: any) {
  const scale = edgy(() => range(0.1, 40), [1, 2, 64, 0.01]);
  const corners = { radii: corner.cornerRadii(d), smoothing: corner.cornerSmoothing(d), inset: corner.cornerInset(d),
    uniform: corner.isUniformCircular(d), svg: corner.roundedRectPath(d) };
  const geometry = shapeRasterGeometry(d, scale);
  canvases = [];
  let result: any, error: string | undefined;
  try { result = rasterShape(d, scale); } catch (e) {
    if (!(e instanceof IndexSizeError)) throw e;
    error = 'IndexSizeError';
  }
  const canvas = canvases[0]!;
  return {
    d, scale, corners, geometry,
    raster: error ? { error } : {
      width: canvas.width, height: canvas.height, w: result.w, h: result.h, anchorX: result.anchorX, anchorY: result.anchorY,
      events: canvas.context.events,
    },
  };
}

/* ── editable paths ── */
let ids = 0;
function vectorPath(index: number, count: number): any {
  const n = pick([0, 1, 2, 2, 3, 3, 4, 5]);
  const vertices = Array.from({ length: n }, () => {
    const tangent = chance(0.5);
    const t = () => (tangent ? edgy(() => range(-80, 80), [NaN, 0]) : 0);
    return { id: `v${ids++}`, p: { x: edgy(() => range(-300, 300), [NaN, Infinity]), y: range(-300, 300), inX: t(), inY: t(), outX: t(), outY: t() } };
  });
  const p = {
    x: edgy(() => range(-100, 100), ODD), y: range(-100, 100),
    rotation: edgy(() => range(-360, 360), [NaN, 90, 180]),
    scaleX: edgy(() => range(-200, 200), [0, 100, NaN]), scaleY: range(10, 200),
    closed: chance(0.5), fill: '#E8E2CF', fillEnabled: chance(0.75),
    fillOpacity: edgy(() => range(-20, 120), [NaN, 100]),
    stroke: '#ffffff', strokeWidth: edgy(() => range(-2, 20), [0, NaN, Infinity]),
    strokeOpacity: edgy(() => range(0, 100), [NaN, 150]),
    trimStart: chance(0.5) ? 0 : edgy(() => range(-10, 100), [NaN, 100]),
    trimEnd: chance(0.5) ? 100 : edgy(() => range(0, 110), [NaN, 0]),
    trimOffset: chance(0.5) ? 0 : edgy(() => range(-720, 720), [NaN, Infinity, 360]),
    copies: chance(0.5) ? 1 : edgy(() => range(0, 6), [NaN, 0, 300, -4, 2.5]),
    repeatX: edgy(() => range(-50, 50), [NaN]), repeatY: range(-50, 50),
    repeatRotation: edgy(() => range(-90, 90), [NaN, Infinity]),
  };
  const parent = chance(0.5) ? null : chance(0.1) ? 'missing' : `path${int(0, count - 1)}`;
  return { id: `path${index}`, name: 'Path', parent, vertices, p };
}

/** FNV-1a 64 over the little-endian IEEE bits of each coordinate. */
function pointsDigest(points: Array<[number, number]>): string {
  const bytes = new Uint8Array(new Float64Array(points.flat()).buffer);
  let hash = 0xcbf29ce484222325n;
  for (const byte of bytes) hash = BigInt.asUintN(64, (hash ^ BigInt(byte)) * 0x100000001b3n);
  return hash.toString(16).padStart(16, '0');
}

const sameOps = (a: any[], b: any[]) => JSON.stringify(encode(a)) === JSON.stringify(encode(b));

function pathCase(_: unknown, caseIndex: number) {
  const count = int(1, 3);
  const layer = { id: 'layer', d: { paths: Array.from({ length: count }, (_, i) => vectorPath(i, count)) } };
  const scale = edgy(() => range(0.1, 16), [1, 0.01, 100]);
  const paths = layer.d.paths.map((p: any) => {
    const values = pathValues(PM, layer, p, 0);
    const ctx = new Recorder();
    tracePath(ctx as any, values);
    const points = pathPoints(values);
    // Full points for the first cases (readable failures), a bit digest for all.
    return { id: p.id, parent: p.parent, values, ...(caseIndex < 20 ? { points } : {}), pointCount: points.length,
      pointsDigest: pointsDigest(points), trace: ctx.ops, matrix: groupMatrix(PM, layer, p, 0) };
  });
  canvases = [];
  const out = rasterPaths(PM, layer, 0, scale);
  const events = canvases[0]!.context.events.map((e: any) => {
    const index = paths.findIndex((p: any) => sameOps(p.trace, e.path.ops));
    return index < 0 ? e : { ...e, path: { index } };
  });
  return { scale, paths, raster: { width: out.cv.width, height: out.cv.height, w: out.w, h: out.h, anchorX: out.anchorX, anchorY: out.anchorY, selection: out.selection, events } };
}

/* ── masks ── */
const maskGl: any = { uniforms: {} as Record<string, any> };
const { applyMasks, applyPathMasks } = load(
  slice('gl/compositor.ts', 'function applyPathMasks(', 'function applyTrackMatte(') + slice('gl/compositor.ts', 'const MASK_MAX = 8;', '/* ── main render'),
  ['applyMasks', 'applyPathMasks'],
  {
    PM, document: fakeDocument, isProperty, pathValues, tracePath,
    GL: { gl: { disable() {}, enable() {}, BLEND: 0 }, stats: { passes: 0 } },
    evaluatedValue: (_pm: unknown, _layer: unknown, value: unknown) => value,
    scaledWorld: () => maskGl.world,
    grab: () => ({ tex: {} }), bind() {}, clear() {}, free() {}, draw() {}, bindTex() {},
    program: () => ({}), use: () => ({ u() {} }), fullQuad: () => [], m3: (m: number[]) => m,
    is3DLayer: () => false, planeMatrix() {}, outputScale: () => [1, 1], inversePlane() {},
    setU: (_p: unknown, name: string, [value]: [unknown]) => { maskGl.uniforms[name] = value; },
    setI: (_p: unknown, name: string, value: unknown) => { maskGl.uniforms[name] = value; },
    texFor: () => ({}), IDENT: [],
  },
);

/** FRAG_MASK transliterated to JavaScript (double precision). */
function coverage(u: Record<string, any>, lp: [number, number]): { cov: number; edge: number } {
  const g = u.u_g, q = u.u_q;
  let cov = u.u_hasAdd === 1 ? 0 : 1, edge = Infinity;
  for (let i = 0; i < 8; i++) {
    if (i >= u.u_cnt) break;
    if (g[i * 4 + 2] < 0.01 || g[i * 4 + 3] < 0.01) continue;
    let px = lp[0] - g[i * 4], py = lp[1] - g[i * 4 + 1];
    const r = q[i * 4];
    if (Math.abs(r) > 1e-4) { const c = Math.cos(r), s = Math.sin(r); [px, py] = [c * px + s * py, -s * px + c * py]; }
    const hx = Math.max(g[i * 4 + 2] * 0.5, 0.001), hy = Math.max(g[i * 4 + 3] * 0.5, 0.001);
    let d: number;
    if (q[i * 4 + 2] < 0.5) {
      const dx = Math.abs(px) - hx, dy = Math.abs(py) - hy;
      d = Math.hypot(Math.max(dx, 0), Math.max(dy, 0)) + Math.min(Math.max(dx, dy), 0);
    } else d = (Math.hypot(px / hx, py / hy) - 1) * Math.min(hx, hy);
    const f = Math.max(q[i * 4 + 1], 0);
    let a: number;
    if (f > 0.01) { const t = Math.min(1, Math.max(0, (d + f * 0.5) / f)); a = 1 - t * t * (3 - 2 * t); }
    else { a = d <= 0 ? 1 : 0; edge = Math.min(edge, Math.abs(d)); }
    cov = q[i * 4 + 3] < 0.5 ? Math.max(cov, a) : Math.min(cov, 1 - a);
  }
  return { cov, edge };
}

function maskCase() {
  const angle = edgy(() => range(-Math.PI, Math.PI), [0]);
  const sx = edgy(() => range(0.2, 3), [0, 1e-6, NaN]), sy = range(0.2, 3);
  const world: Matrix = [Math.cos(angle) * sx, Math.sin(angle) * sx, -Math.sin(angle) * sy, Math.cos(angle) * sy, range(-500, 500), range(-500, 500)];
  const usePaths = chance(0.3);
  const masks = Array.from({ length: int(0, 11) }, (_, i) => ({
    id: `m${i}`,
    on: chance(0.9) ? (chance(0.5) ? undefined : true) : false,
    p: {
      x: edgy(() => range(-200, 200), [NaN]), y: range(-200, 200),
      w: edgy(() => range(-300, 300), [0, 0.005, NaN]), h: range(-300, 300),
      rotation: edgy(() => range(-180, 180), [0, 90, NaN]),
      feather: edgy(() => range(-5, 40), [0, 0.005, NaN]),
    },
    shape: pick(['rect', 'ellipse', 'rect']),
    mode: pick(['add', 'add', 'subtract']),
    ...(usePaths && chance(0.4) ? { path: vectorPath(0, 1) } : {}),
  }));
  const layer = { id: 'layer', masks };
  const enabled = masks.filter(m => m && m.on !== false && m.p);
  const input = enabled.map(m => ({ ...m.p, shape: m.shape, mode: m.mode, ...(m.path ? { path: pathValues(PM, layer, m.path, 0, `mp.${m.id}`) } : {}) }));
  maskGl.world = world;
  maskGl.uniforms = {};
  canvases = [];
  const result: any = { world, masks: input };
  if (!enabled.length) return result;
  if (enabled.some(m => m.path)) {
    applyMasks(layer, 0, { tex: {} }, 640, 360);
    const [target, ...items] = canvases;
    result.path = {
      startCovered: target!.context.events.some((e: any) => e.op === 'fillRect'),
      masks: items.map(item => {
        const fill = item.context.events.find((e: any) => e.op === 'fill');
        const drawn = target!.context.events.find((e: any) => e.op === 'drawImage' && e.canvas === item.id);
        return { ctm: fill.ctm, ops: fill.path.ops, filter: drawn.filter, composite: drawn.composite };
      }),
    };
  } else {
    applyMasks(layer, 0, { tex: {} }, 640, 360);
    const u = maskGl.uniforms;
    if (!('u_inv' in u)) result.parametric = { skipped: true };
    else {
      const samples = Array.from({ length: 12 }, () => {
        const at: [number, number] = [range(-350, 350), range(-350, 350)];
        return { at, ...coverage(u, at) };
      });
      result.parametric = { inv: u.u_inv, cnt: u.u_cnt, hasAdd: u.u_hasAdd, g: u.u_g, q: u.u_q, samples };
    }
  }
  return result;
}

/* ── encoding ── */
function encode(value: any): any {
  if (typeof value === 'number') return Number.isFinite(value) ? (Object.is(value, -0) ? '-0' : value) : String(value);
  if (Array.isArray(value)) return value.map(encode);
  if (value && typeof value === 'object') return Object.fromEntries(Object.entries(value).filter(([, v]) => v !== undefined).map(([k, v]) => [k, encode(v)]));
  return value;
}

const shapes = [
  ...[
    { shape: 'rect', w: 100, h: 50, radius: 10, smoothing: 0 },
    { shape: 'rect', w: 200, h: 200, radius: 40, smoothing: 60, stroke: 4 },
    { shape: 'rect', w: 120, h: 80, independentCorners: true, radiusTL: 0, radiusTR: 30, radiusBR: 60, radiusBL: 5, smoothing: 100 },
    { shape: 'star', w: 100, h: 100, points: 5 },
    { shape: 'polygon', w: 90, h: 60, points: 6, stroke: 3 },
    { shape: 'ellipse', w: 300, h: 100 },
    { shape: 'line', w: 400, h: 10 },
    { shape: 'rect', w: 10, h: 10, radius: -5 },
    { shape: 'ellipse', w: -10, h: 10 },
  ].map(d => ({ color: '#336699', strokeColor: '#ff8800', ...d })),
  ...Array.from({ length: 450 }, shapeContent),
].map(shapeCase);
const paths = Array.from({ length: 150 }, pathCase);
const masks = Array.from({ length: 220 }, maskCase);

/* ── trigonometry probe ──
 * bun runs JavaScriptCore, whose Math.cos and Math.sin come from the
 * platform libm; Electron's V8 uses its own implementation. Record both (V8
 * through node, when it is installed) over the angles the geometry uses, so
 * the Rust test can measure how often each agrees with the libm crate. */
const angles = Array.from({ length: 1000 }, (_, i) => (i < 400 ? range(-360, 360) * Math.PI / 180 : i < 800 ? i * range(-90, 90) * Math.PI / 180 : range(-1e4, 1e4)));
const trig: any = { angles, jsc: { cos: angles.map(Math.cos), sin: angles.map(Math.sin), tan: angles.map(Math.tan) } };
const node = Bun.which('node');
const hypotPairs = Array.from({ length: 4000 }, () => [range(-1e3, 1e3) * (chance(0.1) ? 1e-6 : 1), range(-1e3, 1e3) * (chance(0.1) ? 1e6 : 1)]);
hypotPairs.push([0, -0], [NaN, Infinity], [NaN, 1], [-Infinity, 2], [3, 4], [1e300, 1e300], [1e-300, 1e-310]);
const hypotJscDiffers = hypotPairs.filter(([a, b]) => !Object.is(jscHypot(a!, b!), v8Hypot(a!, b!))).length;
let hypotChecked = 'node not installed';
if (node) {
  const run = Bun.spawnSync([node, '-e', `const p=JSON.parse(require('fs').readFileSync(0,'utf8'));process.stdout.write(JSON.stringify(p.map(([a,b])=>String(Math.hypot(a,b)))))`], { stdin: Buffer.from(JSON.stringify(encode(hypotPairs)).replace(/"(NaN|-?Infinity|-0)"/g, (_, v) => (v === '-0' ? '-0' : v === 'NaN' ? '0' : v === 'Infinity' ? '1e999' : '-1e999'))) });
  if (run.exitCode !== 0) throw new Error(run.stderr.toString());
  const v8 = JSON.parse(run.stdout.toString()) as string[];
  // JSON cannot carry NaN; the edge pairs above with NaN were sent as 0, so compare against the same inputs.
  const sent = hypotPairs.map(([a, b]) => [Number.isNaN(a) ? 0 : a!, Number.isNaN(b) ? 0 : b!]);
  const bad = sent.filter(([a, b], i) => String(v8Hypot(a!, b!)) !== v8[i]);
  if (bad.length) throw new Error(`V8 Math.hypot transliteration differs from node on ${bad.length} pairs, e.g. ${JSON.stringify(bad[0])}`);
  hypotChecked = `identical to node's V8 on ${sent.length} pairs`;
}
if (node) {
  const run = Bun.spawnSync([node, '-e', `const a=JSON.parse(require('fs').readFileSync(0,'utf8'));process.stdout.write(JSON.stringify({version:process.versions.v8,cos:a.map(Math.cos),sin:a.map(Math.sin),tan:a.map(Math.tan)}))`], { stdin: Buffer.from(JSON.stringify(angles)) });
  if (run.exitCode === 0) trig.v8 = JSON.parse(run.stdout.toString());
}
trig.jsc.version = `bun ${Bun.version}`;

const out = path.join(import.meta.dir, 'geometry-vectors.json');
writeFileSync(out, JSON.stringify(encode({ generatedBy: 'engine/crates/pm-vector/tests/vectors/generate-geometry-vectors.ts', shapes, paths, masks, trig })) + '\n');
const routes = masks.reduce((n: any, m: any) => { n[m.path ? 'path' : m.parametric?.skipped ? 'skipped' : m.parametric ? 'parametric' : 'none']++; return n; }, { path: 0, parametric: 0, skipped: 0, none: 0 });
console.log(`Math.hypot: V8 algorithm installed, ${hypotChecked}; JSC's own differs on ${hypotJscDiffers} of ${hypotPairs.length} pairs`);
console.log(`trig probe: ${trig.jsc.version}${trig.v8 ? `, V8 ${trig.v8.version}` : ', no node (V8 values omitted)'}`);
console.log(`wrote ${shapes.length} shapes (${shapes.filter(s => s.raster.error).length} throw), ${paths.length} path layers, ${masks.length} mask sets ${JSON.stringify(routes)}`);
