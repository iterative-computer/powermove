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
import { LAYER3D_DEFINITIONS, compositionScene } from '../core/scene3d/layers';
import { createScene, parseScene, SCENE3D_DEFINITION } from '../core/scene3d/schema';
import { editScene } from '../core/scene3d/operations';
import { setViewportMode, orbitViewStep, enterLocalView, exitLocalView, toggleViewProjection } from '../core/scene3d/viewport';
import { loadGltf } from '../core/scene3d/import-model';
import { compositionRuntime } from '../core/scene3d/service';
import { runtimeBoxes } from '../core/scene3d/navigation';
import { renderCamera, runtimeCameraFacts } from '../core/scene3d/render-camera';
import { targetScene } from '../core/scene3d/engine-targets';
import { objectGeometry, runtimeObjectMesh, runtimeCursorHit, cursorHit } from '../core/scene3d/engine-edit';
import { meshFromGeometry } from '../core/scene3d/editmesh';
import { resolveTargets, deltaCommands } from '../core/scene3d/targets';
import * as THREE from 'three';
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

async function editor(mode = 'audit', strict = false) {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing', 'gl/compositor');
  PM.layerDefinition = (id: string) => Object.values(LAYER3D_DEFINITIONS).includes(id as any)
    ? { id, label: '3D', version: 1, params: [], defaults: {}, renderer: { kind: 'layer3d' } }
    : id === SCENE3D_DEFINITION ? { id, label: '3D Scene', version: 1, params: [], defaults: { scene: createScene() }, renderer: { kind: 'scene3d' } }
      : id === 'powermove.3d.obj-model' ? { id, label: 'Model', version: 1, params: [], defaults: {}, renderer: { kind: 'mesh', assetField: 'assetId' } }
        // A user's own mesh definition (amendment 24): its asset field is
        // the registry's, not the built-in's.
        : id === 'acme.turntable' ? { id, label: 'Turntable', version: 1, params: [], defaults: {}, renderer: { kind: 'mesh', assetField: 'model' } }
          : null;
  // The asset store's shape (`legacy/gl/raster.ts`): records by id.
  const store = new Map<string, any>();
  PM.assets = { map: store, get: (id: string) => store.get(id) ?? null, set: (id: string, a: any) => store.set(id, a), delete: (id: string) => store.delete(id) };
  const doc: EngineDoc = installEngineDoc(PM, { mode: parseMode(mode), loadModule, sink: () => {}, auditMs: 1e9, strict });
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
  let text: Uint8Array = new TextEncoder().encode(JSON.stringify(json));
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
  // A third-party mesh definition layer; its asset sits in its own field.
  const turntable = PM.mkLayer('extension', { name: 'Turntable' });
  turntable.d = { definition: 'acme.turntable', data: { model: 'cube', assetId: 'missing-field' }, params: { background: PM.P(null), size: PM.P(2.4), yaw: PM.P(10, { kf: [PM.KF(0, 10), PM.KF(3, 200)] }) } };
  PM.addLayer(legacy, 0);
  PM.addLayer(turntable, 0);
  PM.addLayer(model, 0);
  if (invalidBackground) PM.addLayer(invalid, PM.proj.layers.length);
  PM.touch();
}

/** What the picks hit, by kind (the suite must not pass on misses alone). */
const hits = { object: 0, scene: 0, thirdParty: 0, other: 0, none: 0, threw: 0 };
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
        else if (hit.type === 'extension') { hits.scene++; if (hit.d?.definition === 'acme.turntable') hits.thirdParty++; }
        else hits.other++;
      } catch (error) { hits.threw++; const m = String((error as Error)?.message).slice(0, 60); thrown[m] = (thrown[m] ?? 0) + 1; /* both throw; compared */ }
    }
  }
}

let lastKhronos: string[] = [];
/** Animated model-group recipes: a rocket and a staircase whose parts are stored at the recipe's parameters. */
function addModelGroups(PM: any) {
  const zero = { x: 0, y: 0, z: 0, rx: 0, ry: 0, rz: 0, sx: 1, sy: 1, sz: 1 };
  const part = (id: string, groupId: string, partId: string, source: any, base: any, x: number) => {
    const ok = editScene(PM, { operation: 'add_object', object: { id, source, p: { x: base.x + x, y: base.y, z: base.z } } });
    expect(ok.ok, ok.message).toBe(true);
    PM.L(id).d.data.object.generation = { groupId, partId };
    return { layerId: id, base };
  };
  const radius = 0.6, length = 2.8, nose = 1;
  const rocketParts: Record<string, any> = {
    body: part('rbody', 'rocket', 'body', { primitive: 'cylinder', parameters: { radius, height: length } }, zero, -1.6),
    nose: part('rnose', 'rocket', 'nose', { primitive: 'cone', parameters: { radius, height: nose } }, { ...zero, y: length / 2 + nose / 2 }, -1.6),
    engine: part('rengine', 'rocket', 'engine', { primitive: 'cylinder', parameters: { radiusTop: radius * 0.75, radiusBottom: radius * 0.55, height: radius * 0.7 } }, { ...zero, y: -length / 2 - radius * 0.35 }, -1.6),
    fin_01: part('rfin1', 'rocket', 'fin_01', { primitive: 'box', parameters: { width: 0.1, height: 0.5, depth: 0.5 } }, zero, -1.6)
  };
  const rocket = PM.groupLayers(['rbody', 'rnose', 'rengine', 'rfin1'], 'Rocket');
  rocket.id = 'rocket';
  for (const l of PM.proj.layers) if (l.group === rocket.id || ['rbody', 'rnose', 'rengine', 'rfin1'].includes(l.id)) l.group = 'rocket';
  rocket.d = { ...(rocket.d || {}), modeling: { version: 1, recipe: { kind: 'rocket', parameters: { length, radius, nose, fins: 4, finSize: 0.8 }, modifiers: [] },
    p: { length: PM.P(length, { kf: [PM.KF(0, length), PM.KF(3, 4.2)] }), nose: PM.P(nose, { kf: [PM.KF(1, nose), PM.KF(3, 1.6)] }), finSize: PM.P(0.8, { kf: [PM.KF(0, 0.8), PM.KF(2, 1.4)] }) },
    parts: rocketParts, retired: [] } };
  const stairs: Record<string, any> = {};
  const ids: string[] = [];
  for (let i = 0; i < 4; i++) {
    const angle = i * 30 * Math.PI / 180, id = `step${i}`;
    stairs[`step_00${i}`] = part(id, 'stairs', `step_00${i}`, { primitive: 'box', parameters: { width: 1, height: 0.132, depth: 0.45 } }, { ...zero, x: 1.2 * Math.cos(angle), y: i * 0.22, z: 1.2 * Math.sin(angle) }, 1.6);
    ids.push(id);
  }
  const group = PM.groupLayers(ids, 'Staircase');
  for (const l of PM.proj.layers) if (l.group === group.id) l.group = 'stairs';
  group.id = 'stairs';
  group.d = { ...(group.d || {}), modeling: { version: 1, recipe: { kind: 'staircase', parameters: { steps: 4, radius: 1.2, rise: 0.22, turn: 30, width: 1, depth: 0.45 }, modifiers: [] },
    p: { rise: PM.P(0.22, { kf: [PM.KF(0, 0.22), PM.KF(3, 0.32)] }), turn: PM.P(30, { kf: [PM.KF(0, 30), PM.KF(3, 45)] }) },
    parts: stairs, retired: [] } };
  PM.ProjectIndex?.invalidate?.();
  PM.touch();
}

/** Structural difference with exact numbers (key order ignored), or null. */
function sceneDiff(a: any, b: any, path = ''): string | null {
  if (typeof a === 'number' && typeof b === 'number') return Object.is(a, b) || a === b ? null : `${path}: ${a} != ${b}`;
  if (Array.isArray(a) || Array.isArray(b)) {
    if (!Array.isArray(a) || !Array.isArray(b) || a.length !== b.length) return `${path}: array shape`;
    for (let i = 0; i < a.length; i++) { const d = sceneDiff(a[i], b[i], `${path}[${i}]`); if (d) return d; }
    return null;
  }
  if (a && b && typeof a === 'object' && typeof b === 'object') {
    const keys = new Set([...Object.keys(a), ...Object.keys(b)].filter(k => a[k] !== undefined || b[k] !== undefined));
    for (const k of keys) { const d = sceneDiff(a[k], b[k], `${path}.${k}`); if (d) return d; }
    return null;
  }
  return a === b ? null : `${path}: ${JSON.stringify(a)} != ${JSON.stringify(b)}`;
}

const mismatches = (doc: EngineDoc) => doc.dual!.stats.samples;
const fellBack = (doc: EngineDoc) => Object.entries(doc.api!.stats.fallbacks).filter(([k]) => /^(gl\.pick|gl\.bounds|groups\.bounds|objectBoxes3d):/.test(k));

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
    expect(hits.thirdParty).toBeGreaterThan(5);
    expect(hits.threw).toBeGreaterThan(0);
  }, 600_000);

  it('frames the viewport with the engine boxes three.js computes', async () => {
    let compared = 0, worst = 0, inexact = 0;
    for (const seed of [11, 12, 13]) {
      const { PM, doc } = await editor();
      const random = rng(seed);
      const models = await addModels(PM);
      const ids = populate(PM, random, ['cube', 'wedge', ...models]);
      for (const name of models) editScene(PM, { operation: 'add_object', object: { id: `k${models.indexOf(name)}`, source: { assetId: name }, p: { x: random() * 2 } } });
      PM.touch();
      const all = PM.proj.layers.filter((l: any) => l.d?.data?.object).map((l: any) => l.id);
      for (const t of [0, 0.8, 2.6]) {
        let legacyAsked = 0;
        const native = doc.api!.objectBoxes3d(all, t, () => { legacyAsked++; return runtimeBoxes(PM, all, t); });
        const runtime = compositionRuntime(PM, t);
        // Audit mode asks today's answer once as the reference; no fallback.
        expect(legacyAsked).toBeLessThanOrEqual(1);
        expect(Object.keys(doc.api!.stats.fallbacks).filter(k => k.startsWith('objectBoxes3d:'))).toEqual([]);
        for (const id of all) {
          const object = runtime.objects.get(id);
          const box = object ? new THREE.Box3().expandByObject(object) : null;
          const legacy = box && !box.isEmpty() ? [...box.min.toArray(), ...box.max.toArray()] : null;
          // The dual-answer bounds class (1e-6); posed skinned vertices
          // differ from three.js by an ulp or so of f64.
          expect(native[id] === null, `seed ${seed} ${id} t ${t}`).toBe(legacy === null);
          native[id]?.forEach((v, k) => {
            const d = Math.abs(v - legacy![k]!);
            worst = Math.max(worst, d);
            if (d !== 0) inexact++;
            expect(d, `seed ${seed} ${id} t ${t} [${k}]`).toBeLessThanOrEqual(1e-6);
          });
          compared++;
        }
      }
      expect(ids.length).toBeGreaterThan(0);
    }
    expect(compared).toBeGreaterThan(60);
    if (process.env.PM_GEOMETRY_REPORT) writeFileSync(`${process.env.PM_GEOMETRY_REPORT}.boxes`, JSON.stringify({ compared, worst, inexact }));
  }, 300_000);

  it('evaluates animated model-group recipes like compositionScene and picks their parts', async () => {
    const { PM, doc } = await editor();
    await addModels(PM);
    addModelGroups(PM);
    editScene(PM, { operation: 'add_light', light: { id: 'sun', type: 'sun', p: { x: 1, y: 4, z: 3 } } });
    // A material saved before `doubleSided` existed: parseScene's default (double).
    delete PM.L('rbody').d.data.object.material.doubleSided;
    PM.touch();
    doc.barrier!.flush();
    const c = (doc.api as any).comps();
    for (const t of [0, 0.5, 1.25, 2, 3]) {
      const native = doc.wasm!.query({ op: 'compositionScene', comp: c.comp, open: c.open, t });
      expect(native.ok, JSON.stringify(native)).toBeDefined();
      const legacy = JSON.parse(JSON.stringify(compositionScene(PM, t)));
      expect(sceneDiff(native.ok, legacy), `t ${t}`).toBeNull();
    }
    exercise(PM, doc, rng(21), 200);
    setViewportMode(PM, 'editor');
    orbitViewStep(PM, -25, 10);
    exercise(PM, doc, rng(22), 120);
    expect(mismatches(doc)).toEqual([]);
    expect(fellBack(doc)).toEqual([]);
    expect(hits.object).toBeGreaterThan(0);
  }, 300_000);

  it('answers a pick within a frame on a scene with skinned models', async () => {
    const { PM, doc } = await editor('on');
    const models = await addModels(PM);
    const random = rng(31);
    populate(PM, random, ['cube', 'wedge', ...models]);
    for (const name of models) editScene(PM, { operation: 'add_object', object: { id: `k${models.indexOf(name)}`, source: { assetId: name }, p: { x: (random() - 0.5) * 3, z: -1 } } });
    PM.touch();
    setViewportMode(PM, 'editor');
    const api = doc.api!;
    api.pick(1, 1, 0); // models load once
    const times: number[] = [];
    for (let i = 0; i < 400; i++) {
      // A new time per pick: every skinned model is posed again.
      const t = (i % 40) * 0.05, x = random() * PM.proj.w, y = random() * PM.proj.h;
      const start = performance.now();
      try { api.pick(x, y, t); } catch { /* slot throws are answers too */ }
      times.push(performance.now() - start);
    }
    times.sort((a, b) => a - b);
    const p = (q: number) => times[Math.min(times.length - 1, Math.floor(q * times.length))]!;
    if (process.env.PM_GEOMETRY_REPORT) writeFileSync(`${process.env.PM_GEOMETRY_REPORT}.timing`, JSON.stringify({ models, p50: p(0.5), p95: p(0.95), max: times.at(-1) }));
    expect(fellBack(doc)).toEqual([]);
    expect(p(0.95)).toBeLessThan(33);
  }, 300_000);

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
    expect(doc.wasm!.stats().unanswered).toBe(before! + 1);
    PM.assets.delete('cube');
    exercise(PM, doc, rng(9), 40);
    expect(mismatches(doc)).toEqual([]);
    expect(fellBack(doc)).toEqual([]);
  });

  it('places transform targets where the three.js runtime does, and gestures write the same edits', async () => {
    let compared = 0, commands = 0, worst = 0, inexact = 0;
    // More seeds than angles alone: a 1-ulp world matrix must never round a written edit differently.
    for (const seed of [31, 32, 33, 34, 35, 36, 37, 38, 39, 40]) {
      const { PM, doc } = await editor();
      const random = rng(seed);
      const models = await addModels(PM);
      populate(PM, random, ['cube', 'wedge', ...models]);
      for (const name of models.slice(0, 3)) editScene(PM, { operation: 'add_object', object: { source: { assetId: name }, p: { x: random() * 3 - 1.5 } } });
      if (seed === 33 || seed === 37) addModelGroups(PM);
      // A flat group of 3D objects: its pivot is its members' box center.
      const objects = PM.proj.layers.filter((l: any) => l.d?.data?.object && !l.group).map((l: any) => l.id);
      if (objects.length > 2) PM.groupLayers(objects.slice(0, 2), 'Flat');
      PM.touch();
      const all = PM.proj.layers.filter((l: any) => l.d?.definition?.startsWith('powermove.3d.') || l.type === 'group').map((l: any) => l.id);
      const facts = (list: any[]) => list.map(t => ({ id: t.id, role: t.role, modes: t.modes, world: Array.from(t.world.elements), position: t.position.toArray(), rotation: t.rotation.toArray(), target: t.target?.toArray() ?? null, parentInverse: Array.from(t.parentInverse.elements), orientation: t.orientation.toArray() }));
      for (const t of [0, 1.1, 2.4]) {
        PM.time = t;
        const engineTargets = resolveTargets(PM, all, targetScene(PM)), todayTargets = resolveTargets(PM, all, compositionRuntime(PM, t));
        const engine = facts(engineTargets), today = facts(todayTargets);
        // The same targets, roles, and modes; every number within the
        // evaluation class (pm-eval's world3D can differ from the
        // TypeScript product by an ulp: worst recorded below).
        expect(engine.map(f => [f.id, f.role, f.modes]), `seed ${seed} t ${t}`).toEqual(today.map(f => [f.id, f.role, f.modes]));
        engine.forEach((f: any, i) => {
          for (const k of ['world', 'position', 'rotation', 'target', 'parentInverse', 'orientation'] as const) {
            const a: number[] = f[k] ?? [], b: number[] = (today[i] as any)[k] ?? [];
            expect(a.length).toBe(b.length);
            a.forEach((v, j) => {
              const d = Math.abs(v - b[j]!);
              worst = Math.max(worst, d / Math.max(1, Math.abs(b[j]!)));
              if (d !== 0) inexact++;
              expect(d, `seed ${seed} t ${t} ${f.id} ${k}[${j}]`).toBeLessThanOrEqual(1e-12 * Math.max(1, Math.abs(b[j]!)));
            });
          }
        });
        // What a gesture writes: every target's edits for a move, a turn, and
        // a scale are the same commands (document values are tidied to 1e-9).
        const deltas: Array<[THREE.Matrix4, 'translate' | 'rotate' | 'scale']> = [
          [new THREE.Matrix4().makeTranslation(0.37, -0.21, 0.5), 'translate'],
          [new THREE.Matrix4().makeRotationAxis(new THREE.Vector3(0.3, 1, 0.2).normalize(), 0.61), 'rotate'],
          [new THREE.Matrix4().makeScale(1.3, 0.8, 1.1), 'scale']
        ];
        for (const [delta, mode] of deltas) {
          engineTargets.forEach((target, i) => {
            if (!target.modes.includes(mode)) return;
            expect(deltaCommands(PM, target, delta, mode), `seed ${seed} t ${t} ${target.id} ${mode}`).toEqual(deltaCommands(PM, todayTargets[i]!, delta, mode));
            commands++;
          });
        }
        compared += today.length;
      }
      expect(mismatches(doc)).toEqual([]);
      expect(Object.keys(doc.api!.stats.fallbacks).filter(k => /^(objectWorlds3d|objectBoxes3d):/.test(k))).toEqual([]);
    }
    expect(compared).toBeGreaterThan(30);
    expect(commands).toBeGreaterThan(60);
    console.log(`transform targets: ${compared} compared, ${inexact} inexact numbers, worst relative difference ${worst}; ${commands} gesture edits equal`);
  });

  it('gives Edit Mode the meshes and the 3D cursor the surface points three.js does, exactly', async () => {
    let meshes = 0, rays = 0, hits = 0;
    for (const seed of [41, 42, 43, 44]) {
      const { PM, doc } = await editor();
      const random = rng(seed);
      const models = await addModels(PM);
      const ids = populate(PM, random, ['cube', 'wedge', ...models]);
      for (const name of models) editScene(PM, { operation: 'add_object', object: { source: { assetId: name }, p: { x: random() * 3 - 1.5, z: random() } } });
      if (seed === 44) addModelGroups(PM);
      PM.touch();
      const objects = PM.proj.layers.filter((l: any) => l.d?.data?.object).map((l: any) => l.id);
      for (const t of [0, 1.3, 2.6]) {
        PM.time = t;
        for (const id of [...objects, 'nope']) {
          const today = runtimeObjectMesh(PM, id, t);
          let asked = 0;
          const native = doc.api!.objectMesh3d(id, t, () => { asked++; return today; });
          expect(asked).toBeLessThanOrEqual(1);
          expect(native, `seed ${seed} t ${t} ${id}`).toEqual(today);
          // What Edit Mode makes of it: the same EditMesh.
          const runtime = compositionRuntime(PM, t).objects.get(id) as any;
          const engine = objectGeometry(PM, id, t);
          expect(!!engine, `seed ${seed} ${id}`).toBe(!!runtime?.isMesh);
          if (engine && runtime?.geometry?.getAttribute('position')) expect(meshFromGeometry(engine)).toEqual(meshFromGeometry(runtime.geometry));
          meshes++;
        }
        // Rays from the render camera through a grid of points, and from random origins.
        const camera = renderCamera(PM, t);
        for (let k = 0; k < 40; k++) {
          const ray = new THREE.Ray();
          if (k < 30) {
            const x = (k % 6) / 2.5 - 1, y = Math.floor(k / 6) / 2 - 1;
            ray.origin.setFromMatrixPosition(camera.matrixWorld);
            ray.direction.set(x * 0.8, y * 0.8, 0.5).unproject(camera).sub(ray.origin).normalize();
          } else {
            ray.origin.set((random() - 0.5) * 8, (random() - 0.5) * 6, 6 + random() * 4);
            ray.direction.set((random() - 0.5) * 0.6, (random() - 0.5) * 0.6, -1).normalize();
          }
          let today: number[] | null | undefined, threw: string | null = null;
          try { today = runtimeCursorHit(PM, ray, t); } catch (error) { threw = String((error as Error).message); }
          if (threw !== null) { expect(() => doc.api!.cursorHit3d(ray.origin.toArray(), ray.direction.toArray(), t, () => runtimeCursorHit(PM, ray, t))).toThrow(); continue; }
          const native = doc.api!.cursorHit3d(ray.origin.toArray(), ray.direction.toArray(), t, () => today!);
          expect(native, `seed ${seed} t ${t} ray ${k}`).toEqual(today);
          expect(cursorHit(PM, ray, t)?.toArray() ?? null).toEqual(today);
          rays++;
          if (today) hits++;
        }
      }
      expect(mismatches(doc)).toEqual([]);
      expect(Object.keys(doc.api!.stats.fallbacks).filter(k => /^(objectMesh3d|cursorHit3d):/.test(k))).toEqual([]);
      void ids;
    }
    expect(hits).toBeGreaterThan(50);
    console.log(`edit mode and cursor: ${meshes} meshes, ${rays} rays (${hits} hits) equal`);
  });

  it('gives the viewport the render camera three.js holds, exactly', async () => {
    let compared = 0;
    for (const seed of [21, 22, 23, 24]) {
      const { PM, doc } = await editor();
      const random = rng(seed);
      populate(PM, random, ['cube', 'wedge']);
      // A camera of each projection, animated, sometimes inside a 3D group.
      for (const l of PM.proj.layers.filter((l: any) => l.d?.definition === 'powermove.3d.camera')) PM.Edit.apply({ type: 'delete_layer', target: l.id });
      const ortho = seed % 2 === 0;
      editScene(PM, { operation: 'add_camera', id: 'shot', camera: { ...(ortho ? { projection: 'orthographic' } : {}), p: { x: 1.5, y: 2, z: 7, fov: 35 + seed, zoom: 1.2, targetX: 0.3, targetY: 0, targetZ: 0 } } });
      const shot = PM.proj.layers.find((l: any) => l.d?.definition === 'powermove.3d.camera');
      PM.Edit.apply({ type: 'replace_keyframes', target: shot.id, path: 'position.x', keyframes: [{ time: 0, value: -2 }, { time: 3, value: 2.5 }] });
      if (seed > 22) {
        const group = PM.groupLayers([shot.id], 'Dolly');
        PM.Edit.apply([{ type: 'set_layer', target: group.id, patch: { threeD: true } }, { type: 'set_property', target: group.id, path: 'rotation.y', value: 25 }]);
      }
      PM.touch();
      for (const t of [0, 0.9, 2.2]) {
        let asked = 0;
        const native = doc.api!.camera3d(t, () => { asked++; return runtimeCameraFacts(PM, t); });
        const today = runtimeCameraFacts(PM, t);
        expect(asked).toBeLessThanOrEqual(1);
        expect(native, `seed ${seed} t ${t}`).toEqual(today);
        compared++;
      }
      // The viewport's camera with the engine on is the engine's, matrixWorld exactly.
      PM.time = 0.9;
      const camera = renderCamera(PM);
      expect(Array.from(camera.matrixWorld.elements)).toEqual(runtimeCameraFacts(PM, 0.9).world);
      expect(!!(camera as any).isOrthographicCamera).toBe(ortho);
      expect(mismatches(doc)).toEqual([]);
      expect(Object.keys(doc.api!.stats.fallbacks).filter(k => k.startsWith('camera3d:'))).toEqual([]);
    }
    expect(compared).toBe(12);
  });

  it('names a model three.js holds that the engine cannot load instead of answering with the missing box', async () => {
    const { PM, doc } = await editor();
    await addModels(PM);
    // An object3d without the bytes it was loaded from (and a glTF the
    // engine would reject is the same reply): never the orange box.
    const record = PM.assets.get('cube');
    PM.assets.set('cube', { ...record, object3d: record.object3d.clone() });
    editScene(PM, { operation: 'add_object', object: { id: 'm', source: { assetId: 'cube' } } });
    PM.touch();
    const warn = console.warn;
    const warned: string[] = [];
    console.warn = (...args: unknown[]) => { warned.push(args.join(' ')); };
    try {
      doc.api!.pick(PM.proj.w / 2, PM.proj.h / 2, 0);
      const layer = PM.L('m');
      try { doc.api!.bounds(layer, 0); } catch { /* today's answer, compared */ }
    } finally { console.warn = warn; }
    expect(fellBack(doc).map(([k]) => k).sort()).toEqual(['gl.bounds:unsupported', 'gl.pick:unsupported']);
    expect(warned.some(w => w.includes('cannot load the 3D model cube'))).toBe(true);
  });

  it("with Use New Engine on, never answers a 3D question with three.js: what the engine cannot answer is a named, visible failure", async () => {
    // The renderer's install with the setting on (legacy/bootstrap.ts): mode `on`, strict.
    const { PM, doc } = await editor('on', true);
    const toasts: string[] = [];
    PM.toast = (message: string) => { toasts.push(message); };
    await addModels(PM);
    editScene(PM, { operation: 'add_camera', camera: { p: { x: 0, y: 0, z: 6 } } });
    editScene(PM, { operation: 'add_object', object: { id: 'ok', source: { assetId: 'cube' } } });
    PM.touch();
    let legacy = 0;
    const today = () => { legacy++; return {}; };
    // Answered natively: the three.js closures are never called.
    doc.api!.pick(PM.proj.w / 2, PM.proj.h / 2, 0);
    expect(doc.api!.bounds(PM.L('ok'), 0)).not.toBeNull();
    doc.api!.objectBoxes3d(['ok'], 0, today);
    doc.api!.objectWorlds3d(['ok'], 0, today);
    doc.api!.camera3d(0, () => { legacy++; return runtimeCameraFacts(PM, 0); });
    expect(legacy).toBe(0);
    expect(doc.api!.stats.fallbacks).toEqual({});
    // A model three.js holds that the engine cannot load: refused by name, never three.js.
    const record = PM.assets.get('cube');
    PM.assets.set('cube', { ...record, object3d: record.object3d.clone() });
    PM.touch();
    const warn = console.warn;
    console.warn = () => {};
    try {
      expect(() => doc.api!.pick(PM.proj.w / 2, PM.proj.h / 2, 0)).toThrow(/could not answer gl\.pick \(unsupported\)/);
      expect(() => doc.api!.objectBoxes3d(['ok'], 0, today)).toThrow(/could not answer objectBoxes3d \(unsupported\)/);
    } finally { console.warn = warn; }
    expect(legacy).toBe(0);
    expect(doc.failures().map(f => f.code)).toContain('native-answer-unavailable');
    expect(toasts.some(t => t.includes('could not answer gl.pick (unsupported)'))).toBe(true);
  });
});
