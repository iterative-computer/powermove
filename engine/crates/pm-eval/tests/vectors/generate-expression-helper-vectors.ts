/**
 * Generates tests/vectors/expression-helper-vectors.json by running the
 * expression helpers `sample(layerId, channel, localTime)` and
 * `motionPath(layerId, channel, progress, start, end)` through the real
 * TypeScript evaluator under bun: `legacy/core/anim.ts` (`evalProp`,
 * `motionPathValue`), `legacy/core/expression.ts`, and
 * `core/anim/motion-path.ts` (`createPathTiming`), installed as in
 * `generate-eval-vectors.ts`. `tests/expression_helpers.rs` replays every
 * case through pm-eval with pm-expr.
 *
 * The checked-in file comes from V8 (Electron's, the runtime Powermove
 * ships; Node 24 writes the same bytes). JavaScriptCore's `Math.hypot`
 * differs in the last bit, so a bun run is not the reference:
 *
 *   bun build engine/crates/pm-eval/tests/vectors/generate-expression-helper-vectors.ts \
 *     --target=node --outfile /tmp/gen-helpers.mjs
 *   PM_EXPR_HELPER_OUT=engine/crates/pm-eval/tests/vectors/expression-helper-vectors.json \
 *     ELECTRON_RUN_AS_NODE=1 apps/desktop/node_modules/.bin/electron /tmp/gen-helpers.mjs
 *
 * PM_EXPR_HELPER_SEED draws another set; PM_EXPR_HELPER_OUT writes elsewhere.
 * Numbers are written as the hex of their IEEE-754 bits.
 */
import { writeFileSync } from 'node:fs';
import { install as installEasing } from '../../../../../apps/desktop/src/renderer/src/legacy/core/easing';
import { install as installAnim } from '../../../../../apps/desktop/src/renderer/src/legacy/core/anim';
import { install as installModel } from '../../../../../apps/desktop/src/renderer/src/legacy/core/model';

const OUT = process.env.PM_EXPR_HELPER_OUT ?? new URL('./expression-helper-vectors.json', import.meta.url).pathname;
let seed = Number(process.env.PM_EXPR_HELPER_SEED ?? 0x5a3e17);
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

const view = new DataView(new ArrayBuffer(8));
function hex(x: number): string {
  view.setFloat64(0, x);
  return view.getBigUint64(0).toString(16).padStart(16, '0');
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
function enc(v: unknown): unknown {
  if (v === undefined) return { u: 1 };
  if (v === null) return { z: 1 };
  if (typeof v === 'number') return { n: hex(v) };
  if (typeof v === 'string') return { s: v };
  if (typeof v === 'boolean') return { b: v };
  return { j: JSON.stringify(sortDeep(v)) };
}

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

const EASES = ['linear', 'easeIn', 'easeOut', 'easeInOut', 'hold'];
function key(t: number, v: unknown): any {
  const name = pick(EASES);
  const handles = PM.Ease.handles(name === 'hold' ? 'linear' : name);
  return { i: `k${++uid}`, t, v, eo: handles.eo, ei: handles.ei, hold: name === 'hold' && chance(0.5) };
}
function track(count: number, value: () => unknown): any[] {
  const times = Array.from({ length: count }, () => Math.round(range(-1, 6) * 30) / 30).sort((a, b) => a - b);
  return [...new Set(times)].map(t => key(t, value()));
}
function channel(v: unknown, keys: number, value: () => unknown): any {
  return { v, kf: keys ? track(keys, value) : [], expr: null };
}

const SOURCE_IDS = ['A', 'B', 'C'];
function sourceLayer(id: string): any {
  const p: any = {};
  const moving = () => channel(range(-500, 500), pick([0, 2, 3, 4, 6]), () => range(-800, 800));
  p['position.x'] = moving();
  if (chance(0.9)) p['position.y'] = moving();
  if (chance(0.3)) p['position.z'] = moving();
  p.opacity = channel(100, pick([0, 2, 3]), () => range(0, 100));
  p.rotation = channel(0, pick([0, 3]), () => range(-360, 360));
  const d: any = chance(0.5) ? { color: channel('#ff0000', pick([0, 2]), () => pick(['#00ff00', '#0000ff80', '#123456'])) } : {};
  return { id, name: id, type: 'shape', from: range(-1, 2), dur: 8, on: true, p, d };
}

const SAMPLES = [
  "sample('A', 'position.x', t)", "sample('B', 'position.y', 1.25)", "sample('A', 'opacity')",
  "sample('C', 'rotation', t * 0.5)", "sample('A', 'c.color', t)", "sample('B', 'position.z', t)",
  "sample('missing', 'position.x', 1)", "sample('A', 'nope', 1)", "sample('A', 'position.x', 1/0)",
  "sample('A', 'position.x', 'x')", "sample(A, 'position.x', 1)", "sample() + 1",
  "value + sample('B', 'position.x', t) * 0.1", "sample('A', 'position.x', t) > 0 ? 1 : 0",
];
const PATHS = [
  "motionPath('A', 'position.x', clamp(t / 4, 0, 1), 0, 4)", "motionPath('A', 'position.y', 0.5, 0, 4)",
  "motionPath('B', 'position.x', t / 3, -1, 3)", "motionPath('C', 'position.z', 0.25, 0.5, 5.5)",
  "motionPath('A', 'position.x', 0, 0, 4)", "motionPath('A', 'position.x', 1, 0, 4)",
  "motionPath('A', 'position.y', 1.5, 0, 4)", "motionPath('A', 'position.x', -1, 0, 4)",
  "motionPath('A', 'rotation', 0.5, 0, 4)", "motionPath('A', 'position.x', 0.5, 4, 0)",
  "motionPath('A', 'position.x', 0.5, 0, 1/0)", "motionPath('missing', 'position.x', 0.5, 0, 4)",
  "motionPath('A', 'position.x', 0.5)", "motionPath('A', 'position.x', 0.37, 2, 2.0001)",
  "value + motionPath('B', 'position.y', ease(t / 4, 0, 1), 0, 4)",
];

const comps: any[] = [];
for (let c = 0; c < 24; c++) {
  const layers = SOURCE_IDS.map(sourceLayer);
  // The probe's expression is a placeholder each case replaces (the Rust
  // replay does the same text substitution).
  const probe: any = { id: 'P', name: 'Probe', type: 'shape', from: range(-0.5, 0.5), dur: 8, on: true, p: { 'position.x': { v: 10, kf: [], expr: '__EXPR__' }, opacity: channel(100, 0, () => 0) }, d: {} };
  layers.push(probe);
  const sources = [...SAMPLES, ...PATHS];
  const project = sortDeep({ id: 'X', compId: 'C', compName: 'Main', w: 1920, h: 1080, fps: pick([30, 29.97, 24]), dur: 8, layers, comps: {} });
  const text = JSON.stringify(project);
  const cases: any[] = [];
  for (const source of sources) {
    for (const T of [0, 0.5, 1.75, 3.2, range(-1, 7)]) {
      const proj = JSON.parse(text.replace('"__EXPR__"', JSON.stringify(source)));
      PM.proj = proj;
      PM.scope = [];
      PM.touch();
      const L = proj.layers.find((l: any) => l.id === 'P');
      const prop = L.p['position.x'];
      let out: unknown;
      try { out = enc(PM.ev(L, 'position.x', T)); } catch (e) { out = { throw: (e as Error)?.constructor?.name ?? 'Error' }; }
      cases.push({ source, T: hex(T), out, err: PM.expressionErrors.get(prop) ?? null });
    }
  }
  comps.push({ doc: text, cases });
}
writeFileSync(OUT, JSON.stringify({ comps }) + '\n');
console.log(`wrote ${OUT}: ${comps.length} documents, ${comps.reduce((n, c) => n + c.cases.length, 0)} cases`);
