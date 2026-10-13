/**
 * Generates tests/vectors/eval-vectors.json by running the real TypeScript
 * evaluator under bun: `legacy/core/easing.ts`, `core/anim/temporal-ease.ts`,
 * `legacy/core/temporal-bridge.ts`, and `legacy/core/anim.ts` (installed on a
 * minimal `PM` with `legacy/core/model.ts` and `legacy/core/layer-groups.ts`).
 * The Rust tests (`tests/ts_parity.rs`) replay every case.
 *
 *   bun engine/crates/pm-eval/tests/vectors/generate-eval-vectors.ts
 *
 * The same script runs under V8, and its output must be byte-identical:
 *
 *   bun build engine/crates/pm-eval/tests/vectors/generate-eval-vectors.ts \
 *     --target=node --outfile /tmp/gen.mjs
 *   PM_EVAL_OUT=/tmp/v8.json node /tmp/gen.mjs
 *   cmp /tmp/v8.json engine/crates/pm-eval/tests/vectors/eval-vectors.json
 *
 * Larger local runs: PM_EVAL_SCALE=10 PM_EVAL_OUT=/tmp/eval.json, then
 * PM_EVAL_VECTORS=/tmp/eval.json cargo test -p pm-eval --test ts_parity.
 *
 * Channel addresses are written as `address` (a member named `key` with a
 * dotted value trips the secret scanner's generic-api-key rule).
 *
 * Deterministic: a seeded PRNG drives every random case. Numbers are written
 * as the hex of their IEEE-754 bits so NaN, infinities, and -0 survive.
 *
 * Every key object is deep-sorted before evaluation so JavaScript's
 * insertion order equals the engine's member order (the engine does not keep
 * insertion order; see pm_eval::node), which makes `allProps` order
 * comparable. Tracks are evaluated at increasing times on one clone, like
 * playback; cases where the legacy answer depends on evaluation history are
 * excluded (documented in pm_eval::keyframes).
 */
import { writeFileSync } from 'node:fs';
import { install as installEasing } from '../../../../../apps/desktop/src/renderer/src/legacy/core/easing';
import { install as installAnim } from '../../../../../apps/desktop/src/renderer/src/legacy/core/anim';
import { install as installModel } from '../../../../../apps/desktop/src/renderer/src/legacy/core/model';
import * as TE from '../../../../../apps/desktop/src/renderer/src/core/anim/temporal-ease';

const SCALE = Number(process.env.PM_EVAL_SCALE ?? 1);
const OUT = process.env.PM_EVAL_OUT ?? new URL('./eval-vectors.json', import.meta.url).pathname;

/* ── deterministic randomness ───────────────────────────────────────────── */
let seed = 0x5eed1234;
function rand(): number {
  seed |= 0; seed = (seed + 0x6d2b79f5) | 0;
  let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
  t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
  return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
}
const pick = <T>(items: readonly T[]): T => items[Math.floor(rand() * items.length)]!;
const chance = (p: number) => rand() < p;
const range = (a: number, b: number) => a + (b - a) * rand();
const int = (a: number, b: number) => Math.floor(range(a, b + 1));

/* ── encoding ───────────────────────────────────────────────────────────── */
const view = new DataView(new ArrayBuffer(8));
export function hex(x: number): string {
  view.setFloat64(0, x);
  return view.getBigUint64(0).toString(16).padStart(16, '0');
}
function canonical(value: unknown): string {
  return JSON.stringify(sortDeep(value));
}
function sortDeep<T>(value: T): T {
  if (Array.isArray(value)) return value.map(sortDeep) as T;
  if (value && typeof value === 'object') {
    const out: Record<string, unknown> = {};
    for (const key of Object.keys(value).sort()) out[key] = sortDeep((value as any)[key]);
    return out as T;
  }
  return value;
}
type Enc = { u: 1 } | { z: 1 } | { n: string } | { s: string } | { b: boolean } | { j: string } | { throw: string };
function enc(v: unknown): Enc {
  if (v === undefined) return { u: 1 };
  if (v === null) return { z: 1 };
  if (typeof v === 'number') return { n: hex(v) };
  if (typeof v === 'string') return { s: v };
  if (typeof v === 'boolean') return { b: v };
  return { j: canonical(v) };
}
function run(fn: () => unknown): Enc {
  try { return enc(fn()); } catch (e) { return { throw: (e as Error)?.constructor?.name ?? 'Error' }; }
}

/* ── the legacy evaluator ───────────────────────────────────────────────── */
let uid = 0;
const PM: any = {
  clamp: (v: any, a: any, b: any) => (v < a ? a : v > b ? b : v),
  uid: (prefix = 'id') => `${prefix}${++uid}`,
  scope: [],
};
installEasing(PM);
installAnim(PM);
installModel(PM);
PM.scope = [];

/* ── section: easing ────────────────────────────────────────────────────── */
const UNIT_XS = [-1, -0, 0, 1e-12, 0.001, 0.1, 0.25, 0.3333333333333333, 0.5, 0.75, 0.9, 0.999, 1, 1.5, NaN];
const easing: any[] = [];
for (const name of PM.Ease.names) {
  const p = PM.Ease.PRESETS[name];
  const f = PM.Ease.fn(name);
  const xs = [...UNIT_XS, ...Array.from({ length: 40 * SCALE }, () => rand())];
  easing.push({ name, curve: p.map(hex), xs: xs.map(hex), ys: xs.map(x => hex(f(x))) });
}
for (let i = 0; i < 100 * SCALE; i++) {
  const curve = [rand(), range(-2, 3), rand(), range(-2, 3)];
  if (chance(0.1)) { curve[1] = curve[0]!; curve[3] = curve[2]!; }
  const f = PM.Ease.bezier(curve[0], curve[1], curve[2], curve[3]);
  const xs = [...UNIT_XS, ...Array.from({ length: 12 }, () => rand())];
  easing.push({ name: null, curve: curve.map(hex), xs: xs.map(hex), ys: xs.map(x => hex(f(x))) });
}
const names = [
  { eo: [0.62, 0.05], ei: [0, 1] }, { eo: [0.63, 0.06], ei: [0.01, 0.99] }, { eo: [0.3, 0.3], ei: [0.9, 0.1] },
  ...PM.Ease.names.map((n: string) => PM.Ease.handles(n)),
].map(({ eo, ei }: any) => ({ eo: eo.map(hex), ei: ei.map(hex), name: PM.Ease.nameOf(eo, ei) }));

/* ── section: springs ───────────────────────────────────────────────────── */
const springConfigs: unknown[] = [
  {}, { damping: 18, stiffness: 180, mass: 1 }, { damping: 5 }, { stiffness: 400, mass: 2 },
  { damping: 0.5, stiffness: 50, mass: 0.3 }, { damping: '12' }, { mass: null }, { damping: [7] },
  { mass: 0 }, { damping: 'x' }, true, 3, 'spring', [1, 2], { damping: 40, stiffness: 1000 },
];
for (let i = 0; i < 20 * SCALE; i++) springConfigs.push({ damping: range(0, 60), stiffness: range(1, 1500), mass: range(0.05, 5) });
const SPRING_TS = [-1, -0, 0, 1e-9, 1 / 480, 2 / 480, 0.01, 0.1, 0.5, 1, 2.5, 5.999, 6, 7, 1e9, Infinity, NaN];
const springs = springConfigs.map(cfg => {
  const ts = [...SPRING_TS, ...Array.from({ length: 30 }, () => range(0, 7))];
  return {
    cfg: JSON.stringify(cfg),
    ts: ts.map(hex),
    ys: ts.map(t => hex(Number(PM.Ease.spring(t, cfg)))),
    duration: hex(PM.Ease.springDuration(cfg)),
  };
});

/* ── section: temporal-ease functions ───────────────────────────────────── */
const INTERPS = ['linear', 'bezier', 'hold'] as const;
function typedKey(t: number): any {
  const v = chance(0.1) ? pick([NaN, Infinity, 'x', null]) : chance(0.2) ? 0 : range(-1000, 1000);
  return {
    t, v,
    inInterp: pick(INTERPS), outInterp: pick(INTERPS),
    inEase: { speed: chance(0.2) ? 0 : range(-2000, 2000), influence: range(0.1, 100) },
    outEase: { speed: chance(0.2) ? 0 : range(-2000, 2000), influence: range(0.1, 100) },
    autoBezier: chance(0.2), continuous: chance(0.2),
  };
}
const encKey = (k: any) => ({ ...k, t: hex(k.t), v: typeof k.v === 'number' ? hex(k.v) : null,
  inEase: { speed: hex(k.inEase.speed), influence: hex(k.inEase.influence) },
  outEase: { speed: hex(k.outEase.speed), influence: hex(k.outEase.influence) } });
const temporal: any = { solve: [], segments: [], handles: [], auto: [] };
for (let i = 0; i < 600 * SCALE; i++) {
  const x1 = chance(0.1) ? pick([0, 1, 0.001]) : rand();
  const x2 = chance(0.1) ? pick([0, 1, 0.999]) : rand();
  const x = chance(0.1) ? pick([-0.5, 0, 1e-12, 1, 1.5, NaN]) : rand();
  temporal.solve.push([x1, x2, x, TE.solveCurveParam(x1, x2, x)].map(hex));
}
for (let i = 0; i < 200 * SCALE; i++) {
  const ta = range(-10, 10);
  const tb = chance(0.1) ? ta + pick([0, 1e-10, 1e-8]) : ta + range(0.001, 20);
  const a = typedKey(ta), b = typedKey(tb);
  const times = [ta, tb, (ta + tb) / 2, ta - 1, tb + 1, ...Array.from({ length: 6 }, () => range(ta, tb))];
  temporal.segments.push({
    a: encKey(a), b: encKey(b), times: times.map(hex),
    values: times.map(t => hex(TE.valueAtTime(a, b, t))),
    params: times.map(t => hex(TE.segmentParamAtTime(a, b, t))),
    unit: TE.segmentUnitCurve(a, b)?.map(hex) ?? null,
    points: Object.values(TE.segmentControlPoints(a, b)).flatMap((p: any) => [hex(p.t), hex(p.v)]),
  });
}
for (let i = 0; i < 300 * SCALE; i++) {
  const handle: [number, number] = [chance(0.1) ? pick([NaN, -1, 0, 1, 2]) : rand(), range(-2, 3)];
  const dt = chance(0.1) ? pick([0, 1e-12, -1]) : range(0.001, 10);
  const dv = chance(0.1) ? pick([0, NaN, 1e-12]) : range(-1000, 1000);
  const side = pick(['in', 'out'] as const);
  const ease = TE.easeFromUnitHandle(handle, dt, dv, side);
  const back = TE.unitHandleFromEase(ease, dt, dv, side);
  temporal.handles.push({ handle: handle.map(hex), dt: hex(dt), dv: hex(dv), side,
    ease: [hex(ease.speed), hex(ease.influence)], back: back?.map(hex) ?? null });
}
for (let i = 0; i < 200 * SCALE; i++) {
  const t0 = range(-5, 5);
  const keys = [typedKey(t0), typedKey(t0 + range(0, 3)), typedKey(t0 + range(3, 6))];
  temporal.auto.push({ keys: keys.map(encKey), speed: hex(TE.autoBezierSpeed(keys[0], keys[1], keys[2])) });
}

/* ── section: keyframe tracks (evalKfs) ─────────────────────────────────── */
function ease(): any {
  if (chance(0.1)) return pick([{ speed: 0, influence: 0 }, { speed: 1e6, influence: 100 }, { speed: -1e6, influence: 0.1 }, { speed: 'abc', influence: 50 }]);
  return { speed: chance(0.3) ? 0 : range(-3000, 3000), influence: chance(0.1) ? pick([0, 100, 150, -5]) : range(0.1, 100) };
}
function handle(fallback: number[]): any {
  if (chance(0.25)) return [...fallback];
  if (chance(0.05)) return pick(['x', null, [1], ['0.2', '0.5'], [NaN, 1]]);
  return [rand(), range(-1, 2)];
}
function spring(): any {
  return pick([{}, { damping: range(1, 40), stiffness: range(20, 800), mass: range(0.2, 3) }, true, { mass: 0 }, { damping: '9' }]);
}
function numericKey(t: number, v: unknown, style: string): any {
  const key: any = { t, v };
  if (style === 'native' || (style === 'mixed' && chance(0.5))) {
    key.inInterp = pick(INTERPS); key.outInterp = pick(INTERPS);
    key.inEase = ease(); key.outEase = ease();
    if (chance(0.25)) key.autoBezier = true;
    if (chance(0.2)) key.continuous = true;
    if (chance(0.05)) key.inInterp = 'smooth';
  } else if (style === 'legacy' || style === 'mixed') {
    if (chance(0.7)) key.eo = handle([0, 0]);
    if (chance(0.7)) key.ei = handle([1, 1]);
    if (chance(0.15)) key.hold = true;
    if (chance(0.1)) key.bezierMode = pick(['continuous', 'split']);
  }
  if (chance(0.08)) key.spring = spring();
  return key;
}
function keyTimes(n: number): number[] {
  const times: number[] = [];
  let t = chance(0.2) ? 0 : range(-3, 3);
  for (let i = 0; i < n; i++) {
    times.push(t);
    t += chance(0.08) ? pick([0, 1e-10, 1e-7]) : chance(0.1) ? range(5, 50) : range(0.01, 2);
  }
  return times;
}
function numericValue(): number {
  return chance(0.15) ? pick([0, 1, -1, 100, 1e6, -1e-6]) : chance(0.5) ? int(-500, 500) : range(-1e4, 1e4);
}
const HEX = '0123456789abcdefABCDEF';
const hexColor = (alpha: boolean) => '#' + Array.from({ length: alpha ? 8 : 6 }, () => pick([...HEX])).join('');
function track(kind: string): any[] {
  const n = kind === 'single' ? 1 : int(2, 7);
  const times = keyTimes(n);
  const style = pick(['native', 'legacy', 'mixed', 'bare']);
  switch (kind) {
    case 'numeric':
    case 'single': {
      const flat = chance(0.1);
      const v0 = numericValue();
      return times.map(t => numericKey(t, flat ? v0 : numericValue(), style));
    }
    case 'odd-middle': {
      const keys = times.map(t => numericKey(t, numericValue(), style));
      if (n > 2) keys[int(1, n - 2)]!.v = pick(['5', 'abc', true, null, '', [3], { a: 1 }, ' 7 ']);
      if (chance(0.3)) keys[int(0, n - 1)]!.t = pick(['1', null, true]);
      return keys;
    }
    case 'strings': return times.map(t => numericKey(t, pick(['a', 'Hello', '', '12', 'x y']), style));
    case 'booleans': return times.map(t => numericKey(t, chance(0.5), style));
    case 'colors': return times.map(t => numericKey(t, chance(0.1) ? pick(['#12345', 'red', '#GGGGGG', 5]) : hexColor(chance(0.4)), style));
    case 'mixed': return times.map(t => numericKey(t, pick([numericValue(), 'txt', true, hexColor(false), null]), style));
    case 'string-times': return times.map(t => numericKey(chance(0.5) ? String(t) : t, chance(0.6) ? numericValue() : hexColor(false), style));
    default: throw new Error(kind);
  }
}
function evalTimes(keys: any[]): number[] {
  const ts = keys.map(k => Number(k.t)).filter(Number.isFinite);
  const lo = Math.min(...ts), hi = Math.max(...ts);
  const out = [lo - 1, ...ts, ...ts.map(t => t + 1e-9), ...ts.map(t => t - 1e-12), hi + 1];
  for (let i = 0; i < 8; i++) out.push(range(lo, hi));
  for (let i = 0; i + 1 < ts.length; i++) out.push((ts[i]! + ts[i + 1]!) / 2);
  out.sort((a, b) => a - b);
  out.push(NaN, Infinity, -Infinity, -0);
  return out;
}
const KINDS = ['numeric', 'numeric', 'numeric', 'single', 'odd-middle', 'strings', 'booleans', 'colors', 'colors', 'mixed', 'string-times'];
const tracks: any[] = [];
for (let i = 0; i < 600 * SCALE; i++) {
  const kind = pick(KINDS);
  const keys = sortDeep(track(kind));
  const text = JSON.stringify(keys);
  const color = kind === 'colors' || kind === 'string-times' ? chance(0.85) : chance(0.1);
  const live = JSON.parse(text);
  const times = evalTimes(keys);
  tracks.push({ kind, kf: text, color, times: times.map(hex), out: times.map(t => run(() => PM.evalKfs(live, t, color))) });
}
for (const special of ['[]', '{}', '"abc"', 'null', '[null]', '[5]', '[{"t":0}]', '[{"t":0,"v":1},{"t":1}]']) {
  const live = JSON.parse(special);
  const times = [0, 0.5, NaN];
  tracks.push({ kind: 'special', kf: special, color: false, times: times.map(hex), out: times.map(t => run(() => PM.evalKfs(live, t, false))) });
}

/* ── section: layers (ev, evP, active, resolveContent, findProp, allProps) ── */
const SOURCES = ['value * 2', 't', 'time', 'fps', 'idx', "param('speed')", "param('missing')", "'label'", '1/0', 'value > 0', 'ch', ')(', "value + 'x'", 'layer.from', 'comp.dur'];
function channel(value: unknown, kind?: string): any {
  const prop: any = { v: value, kf: [], expr: null };
  if (chance(0.45)) prop.kf = track(kind ?? (typeof value === 'number' ? 'numeric' : typeof value === 'boolean' ? 'booleans' : 'strings'));
  if (chance(0.08)) prop.expr = pick(SOURCES);
  if (chance(0.02)) prop.expr = 5;
  return prop;
}
const TRANSFORMS = ['position.x', 'position.y', 'anchor.x', 'anchor.y', 'scale.x', 'scale.y', 'rotation', 'skew', 'opacity'];
let layerSerial = 0;
function layer(type: string, groups: string[]): any {
  const id = `L${++layerSerial}`;
  const L: any = { id, name: `Layer ${layerSerial}`, type, from: chance(0.1) ? pick([NaN, '1', null, undefined]) : range(-2, 4), dur: chance(0.05) ? pick([0, -1, 'x']) : range(0.5, 8), p: {} };
  if (L.from === undefined) delete L.from;
  for (const key of TRANSFORMS) if (chance(0.8)) L.p[key] = channel(key.startsWith('scale') || key === 'opacity' ? 100 : 0);
  if (chance(0.2)) L.p['position.z'] = channel(0);
  if (chance(0.03)) L.p.weird = pick([5, 'str', true, { v: 1 }]);
  L.on = chance(0.15) ? channel(true, 'booleans') : chance(0.9);
  if (groups.length && chance(0.4)) L.group = pick(groups);
  const d: any = {};
  switch (type) {
    case 'text':
      d.text = channel('Hello', 'strings'); d.size = channel(48); d.color = channel('#ffffff', 'colors');
      d.font = 'Inter'; d['fontAxis.wght'] = channel(400); d['fontAxis.ab'] = channel(1);
      d.animators = [{ id: 'a1', name: chance(0.5) ? 'Fade' : '', p: { opacity: channel(100), x: channel(0), amount: channel(50) } }];
      d.styles = [{ id: 's1', p: { size: channel(20), color: channel('#ff0000', 'colors') } }];
      break;
    case 'solid': d.color = channel('#336699', 'colors'); d.w = channel(100); d.h = 50; d.radius = channel(4); break;
    case 'shape':
      d.color = channel('#112233', 'colors'); d.strokeColor = channel('#445566ff', 'colors'); d.stroke = channel(2); d.shape = 'rect';
      d.paths = [{ id: 'p1', name: 'Path', parent: null, p: { x: channel(0), y: channel(0), rotation: channel(0), closed: channel(true, 'booleans'), fill: channel('#e8e2cf', 'colors') },
        vertices: [{ id: 'v1', p: { x: channel(0), y: channel(0), inX: channel(0) } }, { id: 'v2', p: { x: channel(10), y: channel(5) } }] }];
      break;
    case 'video': d.trim = channel(0); d.speed = channel(1); d.timeRemap = channel(0); d.fit = 'cover'; break;
    case 'audio': d.gain = channel(0); d.fadeIn = channel(0.5); break;
    case 'shader': d.uniforms = { amount: channel(0.5), tint: channel('#ff00ff', 'colors') }; d.w = channel(320); break;
    case 'extension': d.definition = 'ext.test'; d.params = { speed: channel(1), label: channel('a', 'strings') }; break;
    case 'precomp': d.sourceTime = channel(0); d.trim = 0; d.w = channel(1920); break;
    case 'object':
      L.type = 'extension'; d.definition = 'powermove.3d.object';
      d.data = { object: { source: { primitive: 'box' }, material: { p: { color: channel('#c7c4ff', 'colors'), roughness: channel(0.4), opacity: channel(1) }, shader: { p: { glow: channel(0) } } },
        slots: [{ id: 's1', name: 'Slot', material: { p: { metalness: channel(0.1) }, shader: { p: { tint: channel(1) } } } }] } };
      if (chance(0.5)) d.modeling = { p: { height: channel(2) } };
      break;
    case 'light': L.type = 'extension'; d.definition = 'powermove.3d.light'; d.data = { light: { type: 'sun', p: { intensity: channel(3), color: channel('#ffffff', 'colors') } } }; break;
    case 'camera': L.type = 'extension'; d.definition = 'powermove.3d.camera'; d.data = { camera: { p: { fov: channel(45) } }, environment: { p: { ambient: channel(0.5) } } }; break;
    case 'scene3d':
      L.type = 'extension'; d.definition = 'powermove.3d.scene';
      d.data = { scene: { objects: [{ id: 'o1', name: 'Box', p: { x: channel(0) }, material: { p: { roughness: channel(0.4) } } }], lights: [{ id: 'l1', name: 'Sun', p: { intensity: channel(3) } }], camera: { p: { fov: channel(45) } }, environment: { p: { exposure: channel(1) } } } };
      break;
  }
  L.d = d;
  if (chance(0.5)) L.fx = [{ id: 'fx1', type: 'blur', on: chance(0.3) ? channel(true, 'booleans') : true, p: { amount: channel(5), mix: channel(1) } },
    ...(chance(0.3) ? [{ id: 'fx2', type: 'glow', on: false, p: { radius: channel(10) } }] : [])];
  if (chance(0.4)) L.masks = [{ id: 'mk1', mode: chance(0.3) ? channel('add', 'strings') : 'add', on: chance(0.3) ? channel(true, 'booleans') : true, shape: 'rect',
    p: { feather: channel(0), expansion: channel(0), x: channel(0) }, ...(chance(0.5) ? { path: { p: { x: channel(0) }, vertices: [{ id: 'mv1', p: { x: channel(1) } }] } } : {}) }];
  if (chance(0.3)) L.transitionIn = { type: 'fade', dur: 0.5, p: { amount: channel(1), softness: channel(0.2) } };
  if (chance(0.2)) L.transitionOut = { type: 'wipe', dur: 0.5, p: { angle: channel(90) } };
  for (const key of ['blend', 'mblur', 'matteMode']) if (chance(0.1)) L[key] = channel(key === 'mblur' ? true : 'normal', key === 'mblur' ? 'booleans' : 'strings');
  return L;
}
const BOGUS = ['nope', 'm.zz.x', 'm.mk1.nope', 'm.', 'fx9.amount', 'fx1', 'fx1.$enabled', 'c.nothing', 'u.none', 'x.none', 'transitionIn.p.nope', 'transitionIn.p.1x', 'g.none.x', 'g.p1.v.v9.x', 'o.none.x', 'light.none', 'slots.none.x', 'l.blend', 'l.on', 'model.none'];
const LAYER_TYPES = ['solid', 'shape', 'text', 'image', 'video', 'audio', 'shader', 'extension', 'precomp', 'null', 'object', 'light', 'camera', 'scene3d'];
const comps: any[] = [];
for (let c = 0; c < 8 * SCALE; c++) {
  const groupIds: string[] = [];
  const layers: any[] = [];
  for (let g = 0; g < int(0, 3); g++) {
    const group = layer('group', groupIds);
    group.type = 'group';
    groupIds.push(group.id);
    layers.push(group);
  }
  for (let i = 0; i < int(2, 7); i++) layers.push(layer(pick(LAYER_TYPES), groupIds));
  if (chance(0.1) && layers.length > 1) layers[1].id = layers[0].id; // a duplicate id
  const project = sortDeep({
    id: 'P', compId: 'C', compName: 'Main', w: 1920, h: 1080,
    fps: chance(0.8) ? pick([30, 29.97, 24, 60]) : 0,
    dur: chance(0.9) ? range(3, 10) : 'x',
    params: chance(0.7) ? { speed: { value: pick([3, 'fast', 0]) } } : undefined,
    layers,
    comps: {},
  });
  const text = JSON.stringify(project);
  const proj = JSON.parse(text);
  PM.proj = proj;
  PM.scope = [];
  PM.touch();
  const cases: any[] = [];
  proj.layers.forEach((L: any, index: number) => {
    const from = Number(L.from), end = from + Number(L.dur);
    const times = [-1, from, end - 1e-7, Number(proj.dur), range(-2, 12)];
    const all = (() => { try { return PM.allProps(L); } catch (e) { return e as Error; } })();
    cases.push({ op: 'all', layer: index, out: all instanceof Error ? { throw: all.constructor.name }
      : all.map((e: any) => ({ address: e.key, label: enc(e.label), group: enc(e.group), prop: enc(e.prop) })) });
    const keys = [...new Set([...(all instanceof Error ? [] : all.map((e: any) => e.key)), ...BOGUS])];
    for (const key of keys) {
      const found = (() => { try { return { v: PM.findProp(L, key) }; } catch (e) { return { e: e as Error }; } })();
      cases.push({ op: 'find', layer: index, address: key, out: 'e' in found ? { throw: found.e!.constructor.name } : enc(found.v ?? null) });
    }
    for (const T of times) {
      for (const key of Object.keys(L.p ?? {}).concat(['missing'])) {
        const out = run(() => PM.ev(L, key, T));
        const prop = L.p?.[key];
        cases.push({ op: 'ev', layer: index, address: key, T: hex(T), out, err: prop && typeof prop === 'object' ? PM.expressionErrors.get(prop) ?? null : null });
      }
      for (const key of keys) {
        if (T === times[0] || T === times[3]) continue;
        let prop: any;
        try { prop = PM.findProp(L, key); } catch { continue; }
        const out = run(() => PM.evP(L, prop, T, key));
        cases.push({ op: 'evP', layer: index, address: key, T: hex(T), out, err: prop && typeof prop === 'object' ? PM.expressionErrors.get(prop) ?? null : null });
      }
      cases.push({ op: 'active', layer: index, T: hex(T), out: run(() => PM.active(L, T)) });
      const content = (() => { try { return PM.resolveContent(L, T); } catch (e) { return e as Error; } })();
      // Members passed through unevaluated are the live objects, which the
      // legacy evaluator has mutated by now (adopted keys); the engine never
      // mutates the document, so they are compared in their stored form.
      const stored = JSON.parse(text).layers[index]?.d ?? {};
      cases.push({ op: 'content', layer: index, T: hex(T), out: content instanceof Error ? { throw: content.constructor.name }
        : Object.fromEntries(Object.keys(content).sort().map(k => [k, enc(content[k] === L.d?.[k] && content[k] && typeof content[k] === 'object' ? stored[k] : content[k])])) });
    }
  });
  comps.push({ doc: text, cases });
}

writeFileSync(OUT, JSON.stringify({ generator: 'generate-eval-vectors.ts', scale: SCALE, easing, names, springs, springTableLength: Math.ceil(6 / (1 / 480)), temporal, tracks, comps }));
console.log(`wrote ${OUT}: ${easing.length} curves, ${springs.length} springs, ${tracks.length} tracks, ${comps.length} compositions`);
