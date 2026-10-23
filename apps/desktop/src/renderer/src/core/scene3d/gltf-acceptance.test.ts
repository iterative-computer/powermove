/* Which glTF files today's importer accepts (`loadGltf`: three.js
 * GLTFLoader plus the import budget) against the engine's two glTF readers:
 * the importer behind `assets.get().object3d` (`importModel`,
 * pm-geom3d `model_import`) and the renderer's and picks' reader
 * (`setModel`, pm-geom3d `gltf`). A file today's importer refuses never
 * becomes a model (the orange missing-model box); the engine draws that box
 * for a file it does not load, which equals today's only while every file
 * today loads, the engine loads too. Each case is a hand-made file at a
 * boundary either reader checks. Build pm-wasm first. */
import { existsSync, readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { imports, PmWasm } from '../../engine-doc/wasm';
import { loadGltf, disposeModel } from './import-model';
import { object3dFromEngine } from './compat-object3d';

const MODULE = fileURLToPath(new URL('../../../public/engine/pm_wasm.wasm', import.meta.url));
const REQUIRED = process.env.PM_WASM_REQUIRED === '1';

type Json = Record<string, any>;
const f32 = (v: number[]) => new Uint8Array(new Float32Array(v).buffer);
const u16 = (v: number[]) => { const a = new Uint16Array(v.length + (v.length % 2)); a.set(v); return new Uint8Array(a.buffer); };
const u8 = (v: number[]) => { const a = new Uint8Array(Math.ceil(v.length / 4) * 4); a.set(v); return a; };
const IDENTITY = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1];

/** A triangle (positions, normals, uvs, indices), a two-joint skin's
 * attributes and inverse bind matrices, and an animation's keys, in one
 * data-URI buffer. */
function base(): Json {
  const chunks: [string, Uint8Array][] = [
    ['pos', f32([0, 0, 0, 1, 0, 0, 0, 1, 0])],
    ['nrm', f32([0, 0, 1, 0, 0, 1, 0, 0, 1])],
    ['uv', f32([0, 0, 1, 0, 0, 1])],
    ['idx', u16([0, 1, 2])],
    ['joints', u8([0, 1, 0, 0, 0, 1, 0, 0, 1, 0, 0, 0])],
    ['weights', f32([1, 0, 0, 0, .5, .5, 0, 0, 1, 0, 0, 0])],
    ['ibm', f32([...IDENTITY, ...IDENTITY])],
    ['times', f32([0, 1])],
    ['moves', f32([0, 0, 0, 1, 0, 0])],
    ['badIdx', u16([0, 1, 7])],
    ['sparseIdx', u16([7])],
    ['weights0', f32([1, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0])],
  ];
  const views: Json[] = [], offsets: Record<string, number> = {};
  let length = 0;
  for (const [name, bytes] of chunks) { offsets[name] = views.length; views.push({ buffer: 0, byteOffset: length, byteLength: bytes.byteLength }); length += bytes.byteLength; }
  const all = new Uint8Array(length);
  let at = 0;
  for (const [, bytes] of chunks) { all.set(bytes, at); at += bytes.byteLength; }
  const v = (name: string) => offsets[name]!;
  return {
    asset: { version: '2.0' },
    buffers: [{ byteLength: length, uri: `data:application/octet-stream;base64,${Buffer.from(all).toString('base64')}` }],
    bufferViews: views,
    accessors: [
      { bufferView: v('pos'), componentType: 5126, count: 3, type: 'VEC3', min: [0, 0, 0], max: [1, 1, 0] },
      { bufferView: v('nrm'), componentType: 5126, count: 3, type: 'VEC3' },
      { bufferView: v('uv'), componentType: 5126, count: 3, type: 'VEC2' },
      { bufferView: v('idx'), componentType: 5123, count: 3, type: 'SCALAR' },
      { bufferView: v('joints'), componentType: 5121, count: 3, type: 'VEC4' },
      { bufferView: v('weights'), componentType: 5126, count: 3, type: 'VEC4' },
      { bufferView: v('ibm'), componentType: 5126, count: 2, type: 'MAT4' },
      { bufferView: v('times'), componentType: 5126, count: 2, type: 'SCALAR', min: [0], max: [1] },
      { bufferView: v('moves'), componentType: 5126, count: 2, type: 'VEC3' },
      { bufferView: v('badIdx'), componentType: 5123, count: 3, type: 'SCALAR' },
      { bufferView: v('sparseIdx'), componentType: 5123, count: 1, type: 'SCALAR' },
      { bufferView: v('weights0'), componentType: 5126, count: 3, type: 'VEC4' },
    ],
    meshes: [{ primitives: [{ attributes: { POSITION: 0, NORMAL: 1, TEXCOORD_0: 2 }, indices: 3 }] }],
    nodes: [{ name: 'root', children: [1] }, { name: 'tri', mesh: 0 }],
    scenes: [{ nodes: [0] }],
    scene: 0,
  };
}
/** `base()` with a skinned triangle bound to two joints (nodes 2 and 3). */
function skinned(): Json {
  const g = base();
  g.meshes[0].primitives[0].attributes = { POSITION: 0, NORMAL: 1, JOINTS_0: 4, WEIGHTS_0: 5 };
  g.nodes = [{ name: 'root', children: [1, 2] }, { name: 'tri', mesh: 0, skin: 0 }, { name: 'j0', children: [3] }, { name: 'j1', translation: [0, 1, 0] }];
  g.skins = [{ joints: [2, 3], inverseBindMatrices: 6 }];
  return g;
}
function animated(): Json {
  const g = base();
  g.animations = [{ samplers: [{ input: 7, output: 8 }], channels: [{ sampler: 0, target: { node: 1, path: 'translation' } }] }];
  return g;
}

const cases: [string, () => Json][] = [
  ['a valid triangle', base],
  ['a valid skinned triangle', skinned],
  ['a valid animation', animated],
  ['an explicit arraybuffer buffer type', () => { const g = base(); g.buffers[0].type = 'arraybuffer'; return g; }],
  ['an unknown buffer type', () => { const g = base(); g.buffers[0].type = 'blob'; return g; }],
  ['a scene node that is also a child', () => { const g = base(); g.scenes[0].nodes.push(1); return g; }],
  ['a node that is its own child', () => { const g = base(); g.nodes[1].children = [1]; return g; }],
  ['a node child out of range', () => { const g = base(); g.nodes[0].children = [1, 9]; return g; }],
  ['a node mesh out of range', () => { const g = base(); g.nodes[1].mesh = 4; return g; }],
  ['a node camera out of range', () => { const g = base(); g.nodes[1].camera = 0; return g; }],
  ['a node skin out of range', () => { const g = skinned(); g.nodes[1].skin = 3; return g; }],
  ['a node matrix with 15 values', () => { const g = base(); g.nodes[1].matrix = IDENTITY.slice(0, 15); return g; }],
  ['a skin joint out of range', () => { const g = skinned(); g.skins[0].joints = [2, 9]; return g; }],
  ['too few inverse bind matrices', () => { const g = skinned(); g.skins[0].joints = [2, 3, 0]; return g; }],
  ['joint indices beyond the skin', () => { const g = skinned(); g.skins[0].joints = [2]; g.skins[0].inverseBindMatrices = undefined; return g; }],
  ['a skinned primitive without a skin', () => { const g = skinned(); delete g.nodes[1].skin; delete g.skins; return g; }],
  ['inverse bind matrices that are not MAT4', () => { const g = skinned(); g.skins[0].inverseBindMatrices = 0; return g; }],
  ['a primitive without POSITION', () => { const g = base(); delete g.meshes[0].primitives[0].attributes.POSITION; return g; }],
  ['POSITION as VEC2', () => { const g = base(); g.meshes[0].primitives[0].attributes.POSITION = 2; return g; }],
  ['NORMAL shorter than POSITION', () => { const g = base(); g.accessors[1].count = 2; return g; }],
  ['an index past the vertices', () => { const g = base(); g.meshes[0].primitives[0].indices = 9; return g; }],
  ['an index past the vertices beside a drawable primitive', () => { const g = base(); g.meshes[0].primitives.push({ ...g.meshes[0].primitives[0], indices: 9 }); return g; }],
  ['an accessor past its buffer view', () => { const g = base(); g.accessors[0].count = 4; return g; }],
  ['a buffer view past its buffer', () => { const g = base(); g.bufferViews[0].byteLength = 1 << 20; return g; }],
  ['an accessor component type of doubles', () => { const g = base(); g.accessors[2].componentType = 5130; return g; }],
  ['an unknown accessor type', () => { const g = base(); g.accessors[2].type = 'VEC5'; return g; }],
  ['primitive mode points', () => { const g = base(); g.meshes[0].primitives[0].mode = 0; return g; }],
  ['primitive mode lines', () => { const g = base(); g.meshes[0].primitives[0].mode = 1; return g; }],
  ['primitive mode triangle strip', () => { const g = base(); g.meshes[0].primitives[0].mode = 5; return g; }],
  ['primitive mode 7', () => { const g = base(); g.meshes[0].primitives[0].mode = 7; return g; }],
  ['a material out of range', () => { const g = base(); g.meshes[0].primitives[0].material = 3; return g; }],
  ['an unknown required extension', () => { const g = base(); g.extensionsUsed = ['EXT_unknown']; g.extensionsRequired = ['EXT_unknown']; return g; }],
  ['Draco used', () => { const g = base(); g.extensionsUsed = ['KHR_draco_mesh_compression']; return g; }],
  ['an unknown punctual light type', () => { const g = base(); g.extensionsUsed = ['KHR_lights_punctual']; g.extensions = { KHR_lights_punctual: { lights: [{ type: 'laser' }] } }; g.nodes[0].extensions = { KHR_lights_punctual: { light: 0 } }; return g; }],
  ['a punctual light out of range', () => { const g = base(); g.extensionsUsed = ['KHR_lights_punctual']; g.extensions = { KHR_lights_punctual: { lights: [] } }; g.nodes[0].extensions = { KHR_lights_punctual: { light: 0 } }; return g; }],
  ['an animation target out of range', () => { const g = animated(); g.animations[0].channels[0].target.node = 9; return g; }],
  ['an animation sampler out of range', () => { const g = animated(); g.animations[0].channels[0].sampler = 4; return g; }],
  ['an animation channel without a node', () => { const g = animated(); delete g.animations[0].channels[0].target.node; return g; }],
  ['an unknown animation path', () => { const g = animated(); g.animations[0].channels[0].target.path = 'pointer'; return g; }],
  ['no scenes', () => { const g = base(); delete g.scenes; delete g.scene; return g; }],
  ['an empty scene', () => { const g = base(); g.scenes[0].nodes = []; return g; }],
  ['a scene index out of range', () => { const g = base(); g.scene = 2; return g; }],
  ['a sparse accessor', () => { const g = base(); g.bufferViews.push({ buffer: 0, byteOffset: g.bufferViews[3].byteOffset, byteLength: 2 }); g.accessors[0].sparse = { count: 1, indices: { bufferView: g.bufferViews.length - 1, componentType: 5123 }, values: { bufferView: 0 } }; return g; }],
  ['a sparse index out of range', () => { const g = base(); g.accessors[0].sparse = { count: 1, indices: { bufferView: 10, componentType: 5123 }, values: { bufferView: 0 } }; return g; }],
  ['a scene node listed twice', () => { const g = base(); g.nodes.push({ name: 'second', mesh: 0, translation: [2, 0, 0] }); g.scenes[0].nodes = [0, 2, 0]; return g; }],
  ['a 12-value matrix', () => { const g = base(); g.nodes[1].matrix = IDENTITY.slice(0, 12); return g; }],
  ['a mirrored 15-value matrix', () => { const g = base(); g.nodes[1].matrix = [-1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0]; return g; }],
  ['a 17-value matrix', () => { const g = base(); g.nodes[1].matrix = [...IDENTITY, 5]; return g; }],
  ['a zero weight on a joint beyond the skin', () => { const g = skinned(); g.skins[0].joints = [2]; g.skins[0].inverseBindMatrices = undefined; g.meshes[0].primitives[0].attributes.WEIGHTS_0 = 11; return g; }],
  ['a TEXCOORD shorter than POSITION', () => { const g = base(); g.accessors[2].count = 2; return g; }],
  ['a morph target shorter than POSITION', () => { const g = base(); g.accessors.push({ ...g.accessors[1], count: 2 }); g.meshes[0].primitives[0].targets = [{ POSITION: 12 }]; return g; }],
  ['JOINTS_0 shorter than POSITION', () => { const g = skinned(); g.accessors[4].count = 2; return g; }],
  ['an unknown type on POSITION', () => { const g = base(); g.accessors[0].type = 'VEC5'; return g; }],
];

/** The same document as a GLB: the buffer's data URI becomes the BIN chunk. */
function glb(json: Json): Uint8Array {
  const g = structuredClone(json);
  const uri: string = g.buffers[0].uri;
  const bin = Buffer.from(uri.slice(uri.indexOf(',') + 1), 'base64');
  delete g.buffers[0].uri;
  const text = Buffer.from(JSON.stringify(g));
  const pad = (n: number) => (4 - (n % 4)) % 4;
  const jsonChunk = Buffer.concat([text, Buffer.alloc(pad(text.length), 0x20)]);
  const binChunk = Buffer.concat([bin, Buffer.alloc(pad(bin.length), 0)]);
  const out = Buffer.alloc(12 + 8 + jsonChunk.length + 8 + binChunk.length);
  out.writeUInt32LE(0x46546c67, 0); out.writeUInt32LE(2, 4); out.writeUInt32LE(out.length, 8);
  out.writeUInt32LE(jsonChunk.length, 12); out.writeUInt32LE(0x4e4f534a, 16); jsonChunk.copy(out, 20);
  const at = 20 + jsonChunk.length;
  out.writeUInt32LE(binChunk.length, at); out.writeUInt32LE(0x004e4942, at + 4); binChunk.copy(out, at + 8);
  return new Uint8Array(out);
}

/** A node two nodes list as a child. GLTFLoader attaches it to whichever
 * parent's `loadNode` promise settles last. */
const twoParents = (): Json => { const g = base(); g.nodes.push({ name: 'other', children: [1] }); g.scenes[0].nodes.push(2); return g; };

/** Today's verdict: `loadGltf` (GLTFLoader may never settle on some broken
 * files; such a file never becomes a model). */
async function today(bytes: Uint8Array): Promise<string | null> {
  const timeout = new Promise<string>(resolve => setTimeout(() => resolve('never settles'), 3000));
  const load = loadGltf(new Blob([new Uint8Array(bytes)])).then(m => { disposeModel(m.object3d); return null; }, (e: Error) => String(e?.message ?? e));
  return Promise.race([load, timeout]);
}

describe.skipIf(!REQUIRED && !existsSync(MODULE))('glTF acceptance: today\'s importer and the engine readers', () => {
  it('the engine loads exactly the files today\'s importer loads', async () => {
    const module = await WebAssembly.compile(readFileSync(MODULE));
    // A reader that traps kills its instance: each reader of each case
    // gets a fresh one, and a trap is a failure of this test.
    const fresh = async () => new PmWasm(await WebAssembly.instantiate(module, imports));
    const warn = console.warn, error = console.error;
    console.warn = () => {}; console.error = () => {};
    const g = globalThis as any;
    const hadProgressEvent = 'ProgressEvent' in g;
    // FileLoader reports progress with DOM ProgressEvents (data-URI buffers).
    if (!hadProgressEvent) g.ProgressEvent = class extends Event { constructor(type: string, init: any = {}) { super(type); Object.assign(this, init); } };
    const rows: { name: string; today: string | null; importer: string | null; renderer: string | null }[] = [];
    try {
      for (const [name, make] of cases) {
        for (const container of ['gltf', 'glb'] as const) {
          const bytes = container === 'glb' ? glb(make()) : new TextEncoder().encode(JSON.stringify(make()));
          let importer: string | null = null;
          try { (await fresh()).importModel(bytes); } catch (e) { importer = String((e as Error).message ?? e); }
          let renderer: string | null;
          try { renderer = (await fresh()).setModel('case', 'gltf', bytes); } catch (e) { renderer = `trap: ${String((e as Error).message ?? e)}`; }
          rows.push({ name: `${name} (${container})`, today: await today(bytes), importer, renderer });
        }
      }
    } finally { console.warn = warn; console.error = error; if (!hadProgressEvent) delete g.ProgressEvent; }
    expect(rows.filter(r => /trap|unreachable|dead/i.test(`${r.importer} ${r.renderer}`)).map(r => r.name)).toEqual([]);
    const verdict = (v: string | null) => (v === null ? 'loads' : 'refuses');
    const differ = rows.filter(r => verdict(r.today) !== verdict(r.importer) || verdict(r.today) !== verdict(r.renderer));
    if (process.env.PM_GLTF_ACCEPTANCE_REPORT) console.log(JSON.stringify(rows, null, 1));
    expect(differ.map(r => `${r.name}: today ${r.today ?? 'loads'}; importer ${r.importer ?? 'loads'}; renderer ${r.renderer ?? 'loads'}`)).toEqual([]);
    // The table is not trivially one-sided.
    expect(rows.filter(r => r.today === null).length).toBeGreaterThan(5);
    expect(rows.filter(r => r.today !== null).length).toBeGreaterThan(5);
  }, 120_000);
});

describe.skipIf(!REQUIRED && !existsSync(MODULE))('glTF acceptance: shared child nodes', () => {
  it('loads a two-parent node and attaches it to the same parent as GLTFLoader', async () => {
    const module = await WebAssembly.compile(readFileSync(MODULE));
    const bytes = new TextEncoder().encode(JSON.stringify(twoParents()));
    const g = globalThis as any;
    const hadProgressEvent = 'ProgressEvent' in g;
    if (!hadProgressEvent) g.ProgressEvent = class extends Event { constructor(type: string, init: any = {}) { super(type); Object.assign(this, init); } };
    try {
      expect(await today(bytes)).toBeNull();
    } finally { if (!hadProgressEvent) delete g.ProgressEvent; }
    const data = new PmWasm(new WebAssembly.Instance(module, imports)).importModel(bytes);
    const model = await object3dFromEngine(data, bytes);
    expect(model.object3d.getObjectByName('tri')?.parent?.name).toBe('other');
    disposeModel(model.object3d);
    expect(new PmWasm(new WebAssembly.Instance(module, imports)).setModel('case', 'gltf', bytes)).toBeNull();
  });
});
