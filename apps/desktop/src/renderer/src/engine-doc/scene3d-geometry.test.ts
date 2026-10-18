/*
 * 3D picks, bounds, and model-group bounds through the real pm-wasm module
 * (engine/crates/pm-eval/src/query3d.rs) against today's three.js runtime
 * (`GL.pick`, `GL.bounds`, `modelGroupBounds`), in dual-answer mode: every
 * native answer is compared with the legacy answer, and the native path
 * must never fall back to three.js.
 *
 * Built by engine/crates/pm-wasm/scripts/build.mjs into public/engine/.
 * Khronos sample models are read from the private corpus when present
 * (`Powermove Engine Corpus/khronos-gltf/`), never from the repository.
 */
import { existsSync, readFileSync, writeFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

import { makePM } from '../legacy/__tests__/make-pm';
import { installEngineDoc, type EngineDoc } from './install';
import { parseMode } from './flags';
import { LAYER3D_DEFINITIONS } from '../core/scene3d/layers';
import { createScene, parseScene, SCENE3D_DEFINITION } from '../core/scene3d/schema';
import { editScene } from '../core/scene3d/operations';
import { setViewportMode, orbitViewStep, enterLocalView, exitLocalView, toggleViewProjection } from '../core/scene3d/viewport';
import { loadGltf } from '../core/scene3d/import-model';
import { parseObj } from '../kernel/obj';

const MODULE = fileURLToPath(new URL('../../public/engine/pm_wasm.wasm', import.meta.url));
const built = existsSync(MODULE);
if (!built && process.env.PM_WASM_REQUIRED) throw new Error(`pm-wasm is not built (${MODULE})`);
let compiled: Promise<WebAssembly.Module> | null = null;
const loadModule = () => (compiled ??= WebAssembly.compile(readFileSync(MODULE)));
const KHRONOS = join(homedir(), 'Library/Application Support/Powermove Engine Corpus/khronos-gltf');

function rng(seed: number) {
  return () => {
    seed |= 0; seed = (seed + 0x6d2b79f5) | 0;
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

async function editor() {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing', 'gl/compositor');
  PM.layerDefinition = (id: string) => Object.values(LAYER3D_DEFINITIONS).includes(id as any)
    ? { id, label: '3D', version: 1, params: [], defaults: {}, renderer: { kind: 'layer3d' } }
    : id === SCENE3D_DEFINITION ? { id, label: '3D Scene', version: 1, params: [], defaults: { scene: createScene() }, renderer: { kind: 'scene3d' } }
      : id === 'powermove.3d.obj-model' ? { id, label: 'Model', version: 1, params: [], defaults: {}, renderer: { kind: 'mesh', assetField: 'assetId' } }
        : null;
  // The asset store's shape (`legacy/gl/raster.ts`): records by id.
  const store = new Map<string, any>();
  PM.assets = { map: store, get: (id: string) => store.get(id) ?? null, set: (id: string, a: any) => store.set(id, a), delete: (id: string) => store.delete(id) };
  const doc: EngineDoc = installEngineDoc(PM, { mode: parseMode('audit'), loadModule, sink: () => {}, auditMs: 1e9 });
  PM.proj = PM.mkProject({ name: '3D', fps: 30, dur: 6, w: 1280, h: 720 });
  PM.time = 0;
  expect(await doc.whenReady).toBe(true);
  doc.auditor?.stop();
  return { PM, doc };
}

/** A cube GLB with a second node turning about Y over one second (LINEAR). */
function cubeGlb(): ArrayBuffer {
  const p = [-1, -1, -1, 1, -1, -1, 1, 1, -1, -1, 1, -1, -1, -1, 1, 1, -1, 1, 1, 1, 1, -1, 1, 1];
  const ix = [0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 1, 5, 0, 5, 4, 2, 3, 7, 2, 7, 6, 1, 2, 6, 1, 6, 5, 3, 0, 4, 3, 4, 7];
  const times = [0, 1];
  const s = Math.SQRT1_2;
  const rot = [0, 0, 0, 1, 0, s, 0, s];
  const f32 = (a: number[]) => new Uint8Array(new Float32Array(a).buffer);
  const u16 = (a: number[]) => { const b = new Uint8Array(new Uint16Array(a).buffer); return b.length % 4 ? new Uint8Array([...b, 0, 0]) : b; };
  const parts = [f32(p), u16(ix), f32(times), f32(rot)];
  const offsets: number[] = []; let len = 0;
  for (const b of parts) { offsets.push(len); len += b.length; }
  const bin = new Uint8Array(len); parts.forEach((b, i) => bin.set(b, offsets[i]!));
  const json = {
    asset: { version: '2.0' }, scene: 0, scenes: [{ nodes: [0, 1] }],
    nodes: [{ mesh: 0, translation: [-1.5, 0, 0] }, { name: 'spinner', mesh: 0, translation: [1.5, 0, 0], scale: [0.6, 0.6, 0.6] }],
    meshes: [{ primitives: [{ attributes: { POSITION: 0 }, indices: 1 }] }],
    buffers: [{ byteLength: len }],
    bufferViews: parts.map((b, i) => ({ buffer: 0, byteOffset: offsets[i], byteLength: i === 1 ? ix.length * 2 : b.length })),
    accessors: [
      { bufferView: 0, componentType: 5126, count: 8, type: 'VEC3', min: [-1, -1, -1], max: [1, 1, 1] },
      { bufferView: 1, componentType: 5123, count: ix.length, type: 'SCALAR' },
      { bufferView: 2, componentType: 5126, count: 2, type: 'SCALAR', min: [0], max: [1] },
      { bufferView: 3, componentType: 5126, count: 2, type: 'VEC4' }
    ],
    animations: [{ channels: [{ sampler: 0, target: { node: 1, path: 'rotation' } }], samplers: [{ input: 2, output: 3, interpolation: 'LINEAR' }] }]
  };
  let text = new TextEncoder().encode(JSON.stringify(json));
  const pad = (b: Uint8Array, v: number) => b.length % 4 ? new Uint8Array([...b, ...new Array(4 - b.length % 4).fill(v)]) : b;
  text = pad(text, 0x20);
  const total = 12 + 8 + text.length + 8 + bin.length;
  const out = new Uint8Array(total), view = new DataView(out.buffer);
  view.setUint32(0, 0x46546c67, true); view.setUint32(4, 2, true); view.setUint32(8, total, true);
  view.setUint32(12, text.length, true); view.setUint32(16, 0x4e4f534a, true); out.set(text, 20);
  view.setUint32(20 + text.length, bin.length, true); view.setUint32(24 + text.length, 0x004e4942, true); out.set(bin, 28 + text.length);
  return out.buffer;
}

const OBJ = `# a wedge
v -1 -1 0
v 1 -1 0
v 0 1 0
v 0 0 -1.5
f 1 2 3
f 1 4 2
f 2 4 3
f 3 4 1
`;

/** Untextured Khronos models: skinning, morphs, instancing, sparse accessors, negative scale. */
const KHRONOS_MODELS = ['BoxAnimated.glb', 'RiggedSimple.glb', 'RiggedFigure.glb', 'BrainStem.glb', 'RecursiveSkeletons.glb', 'AnimatedMorphCube.glb', 'MorphPrimitivesTest.glb', 'SimpleInstancing.glb', 'InterpolationTest.glb', 'NegativeScaleTest.glb', 'MetalRoughSpheresNoTextures.glb', 'Fox.glb', 'CesiumMan.glb'];
const loadedKhronos = new Map<string, any>();

async function addModels(PM: any) {
  const loaded = await loadGltf(new Blob([cubeGlb()], { type: 'model/gltf-binary' }));
  PM.assets.set('cube', { id: 'cube', kind: 'model', format: 'glb', ...loaded });
  PM.proj.assets = { ...(PM.proj.assets || {}), cube: { id: 'cube', kind: 'model', format: 'glb', name: 'cube.glb' } };
  const mesh = parseObj(OBJ);
  PM.assets.set('wedge', { id: 'wedge', kind: 'model', format: 'obj', mesh, sourceText: OBJ, vertices: mesh.sourceVertexCount });
  PM.proj.assets.wedge = { id: 'wedge', kind: 'model', format: 'obj', name: 'wedge.obj' };
  const khronos: string[] = [];
  for (const name of KHRONOS_MODELS) {
    const file = join(KHRONOS, name);
    if (!existsSync(file)) continue;
    let loaded = loadedKhronos.get(name);
    if (loaded === undefined) {
      try { loaded = await loadGltf(new Blob([readFileSync(file)], { type: 'model/gltf-binary' })); }
      catch { loaded = null; /* needs a DOM (textures): skipped */ }
      loadedKhronos.set(name, loaded);
    }
    if (!loaded) continue;
    PM.assets.set(name, { id: name, kind: 'model', format: 'glb', ...loaded });
    PM.proj.assets[name] = { id: name, kind: 'model', format: 'glb', name };
    khronos.push(name);
  }
  return khronos;
}

const PRIMITIVES = ['box', 'sphere', 'plane', 'cylinder', 'cone', 'torus', 'capsule', 'icosahedron'];

/** Random 3D compositions: primitives, sources, models, lights, a camera, 2D barriers, groups. */
function populate(PM: any, random: () => number, models: string[]) {
  const pick = <T,>(a: T[]) => a[Math.floor(random() * a.length)]!;
  const ids: string[] = [];
  const count = 6 + Math.floor(random() * 6);
  for (let i = 0; i < count; i++) {
    const id = `o${i}`;
    const roll = random();
    const source = roll < 0.45 ? { primitive: pick(PRIMITIVES) }
      : roll < 0.55 ? { lathe: [[0, -0.5], [0.4, -0.3], [0.5, 0.2], [0.1, 0.6]], segments: 24 }
        : roll < 0.62 ? { extrude: [[-0.5, -0.5], [0.5, -0.5], [0.3, 0.4], [-0.4, 0.5]], depth: 0.4, bevel: random() < 0.5 ? 0.05 : 0 }
          : roll < 0.68 ? { mesh: { positions: [-1, -1, 0, 1, -1, 0, 1, 1, 0, -1, 1, 0], indices: [0, 1, 2, 0, 2, 3], groups: [{ start: 0, count: 3, materialIndex: 0 }, { start: 3, count: 3, materialIndex: 1 }] } }
            : { assetId: pick([...models, 'missing-asset']) };
    const ok = editScene(PM, { operation: 'add_object', object: { id, source, p: { x: (random() - 0.5) * 6, y: (random() - 0.5) * 3, z: (random() - 0.5) * 4, ry: random() * 90 } } });
    expect(ok.ok, ok.message).toBe(true);
    const L = PM.L(id);
    if (random() < 0.4) L.p['position.x'].kf = [PM.KF(0, (random() - 0.5) * 6), PM.KF(4, (random() - 0.5) * 6)];
    if (random() < 0.3) L.p['rotation.x'] = PM.P(Math.round(random() * 180), { kf: [PM.KF(0, 0), PM.KF(3, 120)] });
    if (random() < 0.3) L.p['scale.x'].v = 50 + random() * 150;
    const object = L.d.data.object;
    if (random() < 0.15) object.material.p.opacity.v = 0;
    if (random() < 0.3) object.material.doubleSided = true;
    if ('mesh' in source && random() < 0.7) object.slots = [0, 1].slice(0, random() < 0.85 ? 2 : 1).map(k => ({ id: `s${k}`, name: `S${k}`, material: JSON.parse(JSON.stringify(object.material)) }));
    if ('assetId' in source && random() < 0.4) object.useSourceMaterials = true;
    if (random() < 0.1) L.on = false;
    if (random() < 0.15) L.p.opacity.v = random() < 0.5 ? 0 : 50;
    ids.push(id);
  }
  editScene(PM, { operation: 'add_light', light: { id: 'sun', type: 'sun', p: { x: 1, y: 4, z: 3 } } });
  if (random() < 0.6) editScene(PM, { operation: 'add_camera', id: 'cam', camera: { p: { x: (random() - 0.5) * 4, y: 1 + random() * 2, z: 6 + random() * 3, fov: 30 + random() * 40 } } });
  // A 2D barrier between depth sets, and groups (plain and model groups).
  if (random() < 0.7) {
    const barrier = PM.mkLayer('solid', { name: 'Barrier' });
    barrier.d.w = PM.P(320);
    barrier.d.h = PM.P(180);
    PM.addLayer(barrier, 2 + Math.floor(random() * 3));
  }
  const members = ids.filter(() => random() < 0.35).slice(0, 3);
  if (members.length > 1) PM.groupLayers(members, 'Model group');
  if (random() < 0.2) PM.L(pick(ids)).solo = true;
  PM.ProjectIndex?.invalidate?.();
  PM.touch();
  return ids;
}

/** A legacy scene layer and OBJ model layers (extension scenes without a background). */
function addExtensionScenes(PM: any, invalidBackground = false) {
  const scene = parseScene({
    objects: [
      { id: 'a', name: 'A', source: { primitive: 'torus' }, p: { x: -0.6, rx: { v: 30, kf: [{ t: 0, v: 0 }, { t: 2, v: 90 }], expr: null } } },
      { id: 'b', name: 'B', parent: 'a', source: { primitive: 'cone' }, p: { y: 1 }, material: { doubleSided: false } },
      { id: 'c', name: 'C', source: { assetId: 'wedge' }, p: { x: 1.2, visible: true } },
      { id: 'd', name: 'D', source: { assetId: 'cube' }, p: { z: -1, sx: 0.5, sy: 0.5, sz: 0.5 } }
    ],
    lights: [{ id: 'key', type: 'sun' }],
    environment: { background: null }
  });
  const legacy = PM.mkLayer('extension', { name: 'Legacy scene' });
  legacy.d = { definition: SCENE3D_DEFINITION, data: { scene }, params: {} };
  const model = PM.mkLayer('extension', { name: 'OBJ model' });
  model.d = { definition: 'powermove.3d.obj-model', data: { assetId: 'wedge' }, params: { background: PM.P(null), yaw: PM.P(40, { kf: [PM.KF(0, 0), PM.KF(2, 90)] }) } };
  // An empty background is invalid for parseScene: today every question throws.
  const invalid = PM.mkLayer('extension', { name: 'Invalid background' });
  invalid.d = { definition: 'powermove.3d.obj-model', data: { assetId: 'wedge' }, params: { background: PM.P(''), autoRotate: PM.P(true) }, w: 160, h: 120 };
  PM.addLayer(legacy, 0);
  PM.addLayer(model, 0);
  if (invalidBackground) PM.addLayer(invalid, PM.proj.layers.length);
  PM.touch();
}

/** What the picks hit, by kind (the suite must not pass on misses alone). */
const hits = { object: 0, scene: 0, other: 0, none: 0, threw: 0 };
const thrown: Record<string, number> = {};

function exercise(PM: any, doc: EngineDoc, random: () => number, picks = 120) {
  const api = doc.api!;
  const layers: any[] = PM.proj.layers;
  for (const t of [0, 0.75, 1.5, 3.25]) {
    for (const L of layers) {
      // Both engines throw for scenes parseScene refuses; the comparison is recorded.
      try { api.bounds(L, t); } catch { /* compared */ }
      if (L.type === 'group') try { api.groupBounds(L, t); } catch { /* compared */ }
    }
    for (let i = 0; i < picks; i++) {
      const x = random() * PM.proj.w, y = random() * PM.proj.h;
      try {
        const hit = api.pick(x, y, t);
        if (!hit) hits.none++;
        else if (hit.d?.data?.object) hits.object++;
        else if (hit.type === 'extension') hits.scene++;
        else hits.other++;
      } catch (error) { hits.threw++; const m = String((error as Error)?.message).slice(0, 60); thrown[m] = (thrown[m] ?? 0) + 1; /* both throw; compared */ }
    }
  }
}

let lastKhronos: string[] = [];
const mismatches = (doc: EngineDoc) => doc.dual!.stats.samples;
const fellBack = (doc: EngineDoc) => Object.entries(doc.api!.stats.fallbacks).filter(([k]) => /^(gl\.pick|gl\.bounds|groups\.bounds):/.test(k));

describe.skipIf(!built)('3D picks and bounds with pm-wasm', () => {
  it('answer like the three.js runtime on random compositions, in the camera and editor views', async () => {
    let compared = 0;
    for (const seed of [1, 2, 3, 4, 5]) {
      const { PM, doc } = await editor();
      const random = rng(seed);
      const models = await addModels(PM);
      lastKhronos = models;
      populate(PM, random, ['cube', 'wedge', ...models]);
      addExtensionScenes(PM, seed === 5);
      exercise(PM, doc, random);
      // The editor view: orbit, orthographic, Local View.
      setViewportMode(PM, 'editor');
      orbitViewStep(PM, 35, 20);
      exercise(PM, doc, random, 40);
      toggleViewProjection(PM);
      exercise(PM, doc, random, 40);
      const objects = PM.proj.layers.filter((l: any) => l.type === 'extension' && l.d?.data?.object).map((l: any) => l.id);
      if (enterLocalView(PM, objects.slice(0, 2))) { exercise(PM, doc, random, 40); exitLocalView(PM); }
      setViewportMode(PM, 'camera');
      expect(mismatches(doc), `seed ${seed}`).toEqual([]);
      expect(fellBack(doc), `seed ${seed}`).toEqual([]);
      expect(doc.dual!.fragileBudget().ok, `seed ${seed}`).toBe(true);
      compared += Object.entries(doc.dual!.stats.compared).filter(([k]) => /gl\.pick|gl\.bounds|groups\.bounds/.test(k)).reduce((a, [, n]) => a + n, 0);
    }
    expect(compared).toBeGreaterThan(4000);
    if (process.env.PM_GEOMETRY_REPORT) writeFileSync(process.env.PM_GEOMETRY_REPORT, JSON.stringify({ compared, hits, thrown, khronos: lastKhronos }));
    expect(hits.object).toBeGreaterThan(200);
    expect(hits.scene).toBeGreaterThan(20);
    expect(hits.threw).toBeGreaterThan(0);
  }, 600_000);

  it('asks the renderer for model assets once and follows a relinked asset', async () => {
    const { PM, doc } = await editor();
    await addModels(PM);
    editScene(PM, { operation: 'add_object', object: { id: 'm', source: { assetId: 'cube' } } });
    PM.touch();
    const center = [PM.proj.w / 2, PM.proj.h / 2] as const;
    const hit = doc.api!.pick(center[0], center[1], 0);
    expect(hit?.id).toBe('m');
    expect(doc.wasm!.stats().unanswered).toBeGreaterThan(0);
    const before = doc.wasm!.stats().unanswered;
    doc.api!.pick(center[0], center[1], 0.5);
    expect(doc.wasm!.stats().unanswered).toBe(before);
    // A relinked asset (a new record) is fetched again; a removed one draws the missing box.
    PM.assets.set('cube', { ...PM.assets.get('cube') });
    doc.api!.pick(center[0], center[1], 0.5);
    expect(doc.wasm!.stats().unanswered).toBe(before + 1);
    PM.assets.delete('cube');
    exercise(PM, doc, rng(9), 40);
    expect(mismatches(doc)).toEqual([]);
    expect(fellBack(doc)).toEqual([]);
  });
});
