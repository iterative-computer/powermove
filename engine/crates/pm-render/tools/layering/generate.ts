/**
 * Depth sets and 3D ordering vectors for the native scene pass
 * (docs/engine/scene3d.md, plan section 8): random compositions of 3D object
 * layers, 2D solids, 3D nulls, lights, cameras, plain groups, inactive and
 * invisible layers, and solo, evaluated by today's TypeScript
 * (`legacy/gl/compositor.ts`'s compositing pass, then
 * `core/scene3d/service.ts`: `compositionOrderedLayers`,
 * `compositionDepthIds` over the ordered pass, and the compositor's
 * bottom-of-set rule) on a minimal `PM` with the legacy evaluator installed. `tests/scene3d_layering.rs`
 * plans every document with the native renderer and compares.
 *
 *   bun engine/crates/pm-render/tools/layering/generate.ts
 *
 * Deterministic (seeded); PM_LAYERING_SEED draws another set and
 * PM_LAYERING_OUT writes elsewhere.
 */
import { writeFileSync } from 'node:fs';
import { install as installEasing } from '../../../../../apps/desktop/src/renderer/src/legacy/core/easing';
import { install as installAnim } from '../../../../../apps/desktop/src/renderer/src/legacy/core/anim';
import { install as installModel } from '../../../../../apps/desktop/src/renderer/src/legacy/core/model';
import { compositionDepthIds, compositionOrderedLayers } from '../../../../../apps/desktop/src/renderer/src/core/scene3d/service';
import { layer3DRole } from '../../../../../apps/desktop/src/renderer/src/core/scene3d/layers';
import { compositingLayers } from '../../../../../apps/desktop/src/renderer/src/legacy/gl/compositor';

const CASES = Number(process.env.PM_LAYERING_CASES ?? 80);
const OUT = process.env.PM_LAYERING_OUT ?? new URL('../../tests/fixtures/scene3d-layering.json', import.meta.url).pathname;

let seed = Number(process.env.PM_LAYERING_SEED ?? 0x3d5e7);
function rand(): number {
  seed |= 0; seed = (seed + 0x6d2b79f5) | 0;
  let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
  t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
  return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
}
const chance = (p: number) => rand() < p;
const range = (a: number, b: number) => a + (b - a) * rand();
const int = (a: number, b: number) => Math.floor(range(a, b + 1));
// Quarter units: depths stay distinct enough to be exact in f32 and f64.
const coord = (a: number, b: number) => Math.round(range(a, b) * 4) / 4;

/* ── documents (the shapes of tests/support/doc.rs) ─────────────────────── */
const W = 320, H = 180, DUR = 2;
const ch = (v: any) => ({ v, kf: [], expr: null });
const transform = (x: number, y: number) => ({
  perspective: ch(50), 'position.z': ch(0), 'anchor.z': ch(0), 'scale.z': ch(100), 'rotation.x': ch(0), 'rotation.y': ch(0),
  'orientation.x': ch(0), 'orientation.y': ch(0), 'orientation.z': ch(0), 'anchor.x': ch(0), 'anchor.y': ch(0),
  'position.x': ch(x), 'position.y': ch(y), 'scale.x': ch(100), 'scale.y': ch(100), rotation: ch(0), opacity: ch(100), skew: ch(0),
});
const layer = (id: string, type: string, d: any, x = 0, y = 0): any => ({
  id, type, name: id, from: 0, dur: DUR, on: true, lock: false, shy: false, collapsed: true, threeD: false, solo: false,
  color: '#4C8DFF', blend: 'normal', mblur: false, parent: null, group: null, matteSource: null, p: transform(x, y), fx: [],
  transitionIn: null, transitionOut: null, masks: [], d, locked_intent: {},
});
const object3d = (id: string, [x, y, z]: number[]) => {
  const l = layer(id, 'extension', {
    definition: 'powermove.3d.object', version: 1, w: W, h: H, params: {},
    data: { object: {
      source: { primitive: 'box', parameters: { width: 0.5, height: 0.5, depth: 0.5 } },
      material: { p: { color: ch('#FF4040'), roughness: ch(0.4), metalness: ch(0.1), emissive: ch('#000000'), emissiveIntensity: ch(1), opacity: ch(1) },
        maps: {}, doubleSided: true },
      useSourceMaterials: false, slots: [], castShadow: true, receiveShadow: true,
    } },
  }, x, y);
  l.threeD = true; l.p['position.z'] = ch(z);
  return l;
};
const sun = (id: string, [x, y, z]: number[]) => {
  const l = layer(id, 'extension', {
    definition: 'powermove.3d.light', version: 1, w: 0, h: 0, params: {},
    data: { light: { type: 'sun', castShadow: false, p: {
      targetX: ch(0), targetY: ch(0), targetZ: ch(0), color: ch('#FFFFFF'), intensity: ch(3), distance: ch(0), decay: ch(2),
      angle: ch(45), penumbra: ch(0.4), width: ch(1), height: ch(1) } } },
  }, x, y);
  l.threeD = true; l.p['position.z'] = ch(z);
  return l;
};
const camera = (id: string, [x, y, z]: number[]) => {
  const l = layer(id, 'extension', {
    definition: 'powermove.3d.camera', version: 1, w: 0, h: 0, params: {},
    data: {
      camera: { projection: 'perspective', p: { targetX: ch(0), targetY: ch(0), targetZ: ch(0), fov: ch(45), zoom: ch(1), near: ch(0.01), far: ch(1000) } },
      environment: { p: { ambient: ch(0.5), ambientColor: ch('#FFFFFF'), exposure: ch(1) }, background: null, shadows: false },
    },
  }, x, y);
  l.threeD = true; l.p['position.z'] = ch(z);
  return l;
};
const project = (layers: any[]) => ({
  id: 'Ptest', name: 'test', w: W, h: H, fps: 30, dur: DUR, bg: '#000000',
  backgroundFill: { type: 'solid', angle: 0, stops: [{ id: 's1', color: '#000000', position: 0 }] },
  compId: 'Ptest-root', compName: 'test', markers: [], work: [0, DUR], params: {}, revision: 0, edits: [],
  created: 0, shutter: 0.5, layers, assets: {}, comps: {},
});

/* ── today's evaluator ──────────────────────────────────────────────────── */
let uid = 0;
const PM: any = { clamp: (v: any, a: any, b: any) => (v < a ? a : v > b ? b : v), uid: (p = 'id') => `${p}${++uid}`, scope: [] };
installEasing(PM);
installAnim(PM);
installModel(PM);
PM.scope = [];

/* ── cases ──────────────────────────────────────────────────────────────── */
// The fixture keeps each kind of layer once (`templates`) and every layer as
// its template plus overrides (`[path, value]`); the Rust test expands them
// the same way.
const templates: Record<string, any> = {
  object: object3d('t', [0, 0, 0]),
  light: sun('t', [0, 0, 0]),
  camera: camera('t', [0, 0, 0]),
  solid: layer('t', 'solid', { color: '#00FF00', w: 10, h: 10 }),
  null: layer('t', 'null', {}),
  group: layer('t', 'group', {}),
};
const setPath = (o: any, path: string[], v: any) => {
  let cur = o;
  for (const k of path.slice(0, -1)) cur = cur[k];
  cur[path.at(-1)!] = v;
};
const expand = (spec: { kind: string; set: [string[], any][] }) => {
  const l = JSON.parse(JSON.stringify(templates[spec.kind]));
  for (const [path, v] of spec.set) setPath(l, path, v);
  return l;
};
const cases: any[] = [];
for (let c = 0; c < CASES; c++) {
  const specs: { kind: string; set: [string[], any][] }[] = [];
  const groups = chance(0.3) ? ['g1'] : [];
  let n = 0;
  const count = int(3, 10);
  const at = (x: number, y: number, z: number): [string[], any][] => [
    [['p', 'position.x', 'v'], x], [['p', 'position.y', 'v'], y], [['p', 'position.z', 'v'], z],
  ];
  for (let i = 0; i < count; i++) {
    const r = rand(), id = `l${n++}`;
    const spec: { kind: string; set: [string[], any][] } = { kind: '', set: [[['id'], id], [['name'], id]] };
    if (r < 0.55) { spec.kind = 'object'; spec.set.push(...at(coord(-2, 2), coord(-1.5, 1.5), coord(-4, 3))); }
    else if (r < 0.7) { spec.kind = 'solid'; spec.set.push(...at(int(5, 300), int(5, 170), 0)); }
    else if (r < 0.8) { spec.kind = 'null'; spec.set.push([['threeD'], true], ...at(0, 0, coord(-3, 3))); }
    else { spec.kind = 'light'; spec.set.push(...at(coord(-4, 4), coord(1, 5), coord(-2, 4))); }
    // Inactive at the frame's time, invisible (opacity 0 or nearly), solo.
    if (chance(0.08)) spec.set.push([['from'], 1.5], [['dur'], 0.4]);
    if (chance(0.06)) spec.set.push([['p', 'opacity', 'v'], chance(0.5) ? 0 : 0.05]);
    if (groups.length && chance(0.35)) spec.set.push([['group'], 'g1']);
    specs.push(spec);
  }
  // Solo on one layer in some compositions (any kind: solo on a light or a
  // solid hides every model).
  if (chance(0.2)) specs[int(0, specs.length - 1)]!.set.push([['solo'], true]);
  // The camera somewhere in the stack (or none: the studio camera).
  if (chance(0.85)) {
    specs.splice(int(0, specs.length), 0, { kind: 'camera', set: [[['id'], 'cam'], [['name'], 'cam'], ...at(coord(-3, 3), coord(-1, 3), coord(5, 9))] });
  }
  if (groups.length) {
    const g: { kind: string; set: [string[], any][] } = { kind: 'group', set: [[['id'], 'g1'], [['name'], 'g1']] };
    if (chance(0.3)) g.set.push([['solo'], true]);
    // A group header sits above its first member.
    const first = specs.findIndex(s => s.set.some(([p, v]) => p[0] === 'group' && v === 'g1'));
    specs.splice(first < 0 ? 0 : first, 0, g);
  }
  const doc = project(specs.map(expand));
  const time = 0.5;
  // As `GL.renderProject` runs the open composition: the compositing pass
  // (plain groups flatten into their members), its 3D ordering, which is
  // also the pass `compositionDepthIds` walks, and the bottom of each set.
  PM.proj = doc; PM.scope = []; PM.touch?.();
  PM.curComp = () => PM.proj;
  PM._scene3dRevision = c;
  const pass = compositingLayers(doc.layers, null, () => false);
  const orderedLayers = compositionOrderedLayers(PM, pass, time);
  PM._renderComposition3D = doc;
  PM._renderComposition3DPass = orderedLayers;
  const ordered = orderedLayers.map((l: any) => l.id);
  const sets: Record<string, { set: string[]; background: boolean }> = {};
  for (const l of orderedLayers) {
    if (layer3DRole(l) !== 'object' || !PM.active(l, time)) continue;
    const ids = compositionDepthIds(PM, l, time);
    const bottom = ordered.filter((id: string) => ids.has(id)).at(-1);
    sets[l.id] = { set: [...ids].sort(), background: bottom === l.id };
  }
  PM._renderComposition3D = undefined;
  PM._renderComposition3DPass = undefined;
  cases.push({ layers: specs, time, ordered, sets });
}
const fixture = { generatedBy: 'engine/crates/pm-render/tools/layering/generate.ts', project: project([]), templates, cases };
writeFileSync(OUT, `${JSON.stringify(fixture)}\n`);
console.log(`${cases.length} cases written to ${OUT}`);
