import { describe, expect, it } from 'vitest';
import { existsSync, readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import * as THREE from 'three';
import { imports, PmWasm } from '../../engine-doc/wasm';
import { object3dFromEngine } from './compat-object3d';
import { disposeModel, loadGltf } from './import-model';

const MODULE = fileURLToPath(new URL('../../../public/engine/pm_wasm.wasm', import.meta.url));

type Node = { name: string; children?: number[]; mesh?: number; camera?: number };

function generated(seed: number): Uint8Array {
  let random = seed;
  const next = () => { random = (Math.imul(random, 1664525) + 1013904223) >>> 0; return random; };
  const nodes: Node[] = [
    { name: 'first', children: [2] },
    { name: 'second', children: [2] },
    { name: 'shared', mesh: 0 },
  ];
  const count = 4 + next() % 7;
  for (let i = 3; i < count; i++) {
    nodes.push({ name: `branch${i}`, ...(next() % 3 === 0 ? { mesh: 0 } : {}) });
    const parent = next() % i;
    (nodes[parent]!.children ??= []).push(i);
    if (next() % 5 === 0) (nodes[next() % 2]!.children ??= []).push(i);
  }
  if (next() % 2 === 0) nodes[0]!.mesh = 0;
  if (next() % 2 === 0) nodes[1]!.mesh = 0;
  if (next() % 2 === 0) delete nodes[2]!.mesh;
  if (next() % 3 === 0) nodes[0]!.camera = 0;
  if (next() % 3 === 0) nodes[1]!.camera = 0;
  nodes.push({ name: 'anchor', mesh: 0 });
  return fixture(nodes, [0, 1, nodes.length - 1]);
}

function fixture(nodes: Node[], roots: number[]): Uint8Array {
  const pos = new Float32Array([0, 0, 0, 1, 0, 0, 0, 1, 0]);
  return new TextEncoder().encode(JSON.stringify({
    asset: { version: '2.0' },
    buffers: [{ byteLength: pos.byteLength, uri: `data:application/octet-stream;base64,${Buffer.from(pos.buffer).toString('base64')}` }],
    bufferViews: [{ buffer: 0, byteOffset: 0, byteLength: pos.byteLength }],
    accessors: [{ bufferView: 0, componentType: 5126, count: 3, type: 'VEC3', min: [0, 0, 0], max: [1, 1, 0] }],
    meshes: [{ primitives: [{ attributes: { POSITION: 0 } }] }],
    cameras: [{ type: 'perspective', perspective: { yfov: 1, znear: .1, zfar: 10 } }],
    nodes,
    scenes: [{ nodes: roots }],
  }));
}

function sparseFixture(source: Uint8Array): Uint8Array {
  const document = JSON.parse(new TextDecoder().decode(source));
  const values = new Float32Array([0, 0, 0, 1, 0, 0, 0, 1, 0]);
  const buffer = new Uint8Array(4 + values.byteLength);
  buffer.set([0, 1, 2]); buffer.set(new Uint8Array(values.buffer), 4);
  document.buffers = [{ byteLength: buffer.length,
    uri: `data:application/octet-stream;base64,${Buffer.from(buffer).toString('base64')}` }];
  document.bufferViews = [
    { buffer: 0, byteOffset: 0, byteLength: 3 },
    { buffer: 0, byteOffset: 4, byteLength: values.byteLength },
  ];
  document.accessors = [{ componentType: 5126, count: 3, type: 'VEC3',
    min: [0, 0, 0], max: [1, 1, 0],
    sparse: { count: 3, indices: { bufferView: 0, componentType: 5121 }, values: { bufferView: 1 } },
  }];
  return new TextEncoder().encode(JSON.stringify(document));
}

function glbFixture(source: Uint8Array): Uint8Array {
  const document = JSON.parse(new TextDecoder().decode(source));
  const uri = document.buffers[0].uri as string;
  const bin = Buffer.from(uri.slice(uri.indexOf(',') + 1), 'base64');
  delete document.buffers[0].uri;
  const json = Buffer.from(JSON.stringify(document));
  const jsonLength = (json.length + 3) & ~3;
  const binLength = (bin.length + 3) & ~3;
  const out = Buffer.alloc(12 + 8 + jsonLength + 8 + binLength);
  out.writeUInt32LE(0x46546c67, 0); out.writeUInt32LE(2, 4); out.writeUInt32LE(out.length, 8);
  out.writeUInt32LE(jsonLength, 12); out.writeUInt32LE(0x4e4f534a, 16);
  json.copy(out, 20); out.fill(0x20, 20 + json.length, 20 + jsonLength);
  out.writeUInt32LE(binLength, 20 + jsonLength); out.writeUInt32LE(0x004e4942, 24 + jsonLength);
  bin.copy(out, 28 + jsonLength);
  return new Uint8Array(out);
}

function embeddedImageFixture(cache: 'single' | 'sameSampler' | 'differentSampler'): Uint8Array {
  const document = JSON.parse(new TextDecoder().decode(fixture([
    { name: 'first', mesh: 0, children: [2] },
    { name: 'second', mesh: 1, children: [2] },
    { name: 'shared', mesh: 2 },
  ], [0, 1])));
  const positions = Buffer.from(new Float32Array([-1, -1, 0, 1, -1, 0, 0, 1, 0]).buffer);
  const png = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScLbtAAAAABJRU5ErkJggg==', 'base64');
  const bin = Buffer.concat([positions, png]);
  document.buffers = [{ byteLength: bin.length }, { byteLength: positions.length,
    uri: `data:application/octet-stream;base64,${positions.toString('base64')}` }];
  document.bufferViews = [
    { buffer: 0, byteOffset: 0, byteLength: positions.length },
    { buffer: 0, byteOffset: positions.length, byteLength: png.length },
    { buffer: 1, byteOffset: 0, byteLength: positions.length },
  ];
  document.accessors = [0, 2].map(bufferView => ({ bufferView, componentType: 5126,
    count: 3, type: 'VEC3', min: [-1, -1, 0], max: [1, 1, 0] }));
  document.images = [{ bufferView: 1, mimeType: 'image/png' }];
  document.samplers = cache === 'differentSampler' ? [{}, { wrapS: 33071 }] : [{}];
  document.textures = cache === 'single' ? [{ source: 0, sampler: 0 }] : [
    { source: 0, sampler: 0 }, { source: 0, sampler: cache === 'differentSampler' ? 1 : 0 },
  ];
  document.materials = [{ pbrMetallicRoughness: {
    baseColorTexture: { index: 0 },
    ...(cache === 'single' ? {} : { metallicRoughnessTexture: { index: 1 } }),
  } }];
  document.meshes = [
    { primitives: [{ attributes: { POSITION: 0 }, material: 0 }] },
    { primitives: [{ attributes: { POSITION: 1 } }] },
    { primitives: [{ attributes: { POSITION: 0 } }] },
  ];
  const json = Buffer.from(JSON.stringify(document));
  const jsonLength = (json.length + 3) & ~3;
  const binLength = (bin.length + 3) & ~3;
  const out = Buffer.alloc(12 + 8 + jsonLength + 8 + binLength);
  out.writeUInt32LE(0x46546c67, 0); out.writeUInt32LE(2, 4); out.writeUInt32LE(out.length, 8);
  out.writeUInt32LE(jsonLength, 12); out.writeUInt32LE(0x4e4f534a, 16);
  json.copy(out, 20); out.fill(0x20, 20 + json.length, 20 + jsonLength);
  out.writeUInt32LE(binLength, 20 + jsonLength); out.writeUInt32LE(0x004e4942, 24 + jsonLength);
  bin.copy(out, 28 + jsonLength);
  return new Uint8Array(out);
}

async function withDecodedImages<T>(run: () => Promise<T>): Promise<T> {
  const g = globalThis as any;
  const hadSelf = 'self' in g;
  const previousSelf = g.self;
  const previousBitmap = g.createImageBitmap;
  const fileLoad = THREE.FileLoader.prototype.load;
  const bitmapLoad = THREE.ImageBitmapLoader.prototype.load;
  g.self = g;
  g.createImageBitmap = async () => ({ width: 1, height: 1, close() {} });
  // FileLoader's data-URI buffer and ImageBitmapLoader's already-decoded
  // bitmap both complete as ordered tasks. GLTFLoader's own Promise chains,
  // caches, and Blob-URL creation still run unchanged.
  (THREE.FileLoader.prototype as any).load = function (url: string, onLoad: (bytes: ArrayBuffer) => void) {
    const encoded = url.split(',')[1];
    if (!url.startsWith('data:') || !encoded) throw new Error(`Unexpected buffer URL: ${url}`);
    const bytes = Buffer.from(encoded, 'base64');
    setTimeout(() => onLoad(bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength)), 0);
  };
  (THREE.ImageBitmapLoader.prototype as any).load = function (_url: string, onLoad: (bitmap: ImageBitmap) => void) {
    setTimeout(() => onLoad({ width: 1, height: 1, close() {} } as ImageBitmap), 0);
  };
  try {
    return await run();
  } finally {
    THREE.FileLoader.prototype.load = fileLoad;
    THREE.ImageBitmapLoader.prototype.load = bitmapLoad;
    g.createImageBitmap = previousBitmap;
    if (hadSelf) g.self = previousSelf; else delete g.self;
  }
}

function multiSceneFixture(): Uint8Array {
  const source = fixture([
    { name: 'first', children: [2] },
    { name: 'second', children: [2] },
    { name: 'shared', mesh: 0 },
    { name: 'anchor', mesh: 0 },
  ], [0, 1, 3]);
  const document = JSON.parse(new TextDecoder().decode(source));
  const second = new Float32Array([0, 0, 0, 1, 0, 0, 0, 1.25, 0]);
  document.buffers.push({ byteLength: second.byteLength,
    uri: `data:application/octet-stream;base64,${Buffer.from(second.buffer).toString('base64')}` });
  document.bufferViews.push({ buffer: 1, byteOffset: 0, byteLength: second.byteLength });
  document.accessors.push({ bufferView: 1, componentType: 5126, count: 3,
    type: 'VEC3', min: [0, 0, 0], max: [1, 1.25, 0] });
  document.meshes.push({ primitives: [{ attributes: { POSITION: 1 } }] });
  document.nodes.push({ name: 'delay', mesh: 1 });
  document.scenes.push({ nodes: [2, 4] });
  return new TextEncoder().encode(JSON.stringify(document));
}

function instancedFixture(): Uint8Array {
  const document = JSON.parse(new TextDecoder().decode(fixture([
    { name: 'first', mesh: 0, children: [2] },
    { name: 'second', children: [2] },
    { name: 'shared', mesh: 0 },
  ], [0, 1])));
  document.nodes[0].extensions = { EXT_mesh_gpu_instancing: { attributes: { TRANSLATION: 0 } } };
  document.extensionsUsed = ['EXT_mesh_gpu_instancing'];
  return new TextEncoder().encode(JSON.stringify(document));
}

function sharedPrimitiveFixture(): Uint8Array {
  const document = JSON.parse(new TextDecoder().decode(fixture([
    { name: 'first', mesh: 0, children: [2] },
    { name: 'second', mesh: 1, children: [2] },
    { name: 'shared' },
  ], [0, 1])));
  document.accessors.push({ ...document.accessors[0] });
  document.meshes[0].primitives[0].attributes = { POSITION: 0, NORMAL: 1 };
  document.meshes.push({ primitives: [{ attributes: { NORMAL: 1, POSITION: 0 } }] });
  return new TextEncoder().encode(JSON.stringify(document));
}

function materialFixture(kind: 'standard' | 'physical' | 'unlit'): Uint8Array {
  const document = JSON.parse(new TextDecoder().decode(fixture([
    { name: 'first', mesh: 0, children: [2] },
    { name: 'second', children: [2] },
    { name: 'shared', mesh: 0 },
  ], [0, 1])));
  document.meshes[0].primitives[0].material = 0;
  document.materials = [{ pbrMetallicRoughness: { baseColorFactor: [1, .2, .2, 1] } }];
  if (kind === 'physical') {
    document.materials[0].extensions = { KHR_materials_clearcoat: { clearcoatFactor: .5 } };
    document.extensionsUsed = ['KHR_materials_clearcoat'];
  } else if (kind === 'unlit') {
    document.materials[0].extensions = { KHR_materials_unlit: {} };
    document.extensionsUsed = ['KHR_materials_unlit'];
  }
  return new TextEncoder().encode(JSON.stringify(document));
}

function animatedOffSceneFixture(): Uint8Array {
  const document = JSON.parse(new TextDecoder().decode(fixture([
    { name: 'first', children: [2] },
    { name: 'second', children: [2] },
    { name: 'shared', mesh: 0 },
    { name: 'anchor', mesh: 0 },
    { name: 'animatedParent', children: [2] },
  ], [0, 1, 3])));
  const times = new Float32Array([0, 1]);
  const translations = new Float32Array([0, 0, 0, 0, 0, 0]);
  const extra = Buffer.concat([Buffer.from(times.buffer), Buffer.from(translations.buffer)]);
  document.buffers.push({ byteLength: extra.length,
    uri: `data:application/octet-stream;base64,${extra.toString('base64')}` });
  document.bufferViews.push({ buffer: 1, byteOffset: 0, byteLength: times.byteLength },
    { buffer: 1, byteOffset: times.byteLength, byteLength: translations.byteLength });
  document.accessors.push({ bufferView: 1, componentType: 5126, count: 2, type: 'SCALAR' },
    { bufferView: 2, componentType: 5126, count: 2, type: 'VEC3' });
  document.animations = [{ samplers: [{ input: 1, output: 2 }],
    channels: [{ sampler: 0, target: { node: 4, path: 'translation' } }] }];
  return new TextEncoder().encode(JSON.stringify(document));
}

async function parentOf(bytes: Uint8Array): Promise<string | null> {
  const g = globalThis as any;
  const hadProgressEvent = 'ProgressEvent' in g;
  if (!hadProgressEvent) g.ProgressEvent = class extends Event { constructor(type: string, init: any = {}) { super(type); Object.assign(this, init); } };
  try {
    const model = await loadGltf(new Blob([new Uint8Array(bytes)]));
    const parent = model.object3d.getObjectByName('shared')?.parent?.name ?? null;
    disposeModel(model.object3d);
    return parent;
  } finally {
    if (!hadProgressEvent) delete g.ProgressEvent;
  }
}

describe('three r180 shared-node promise order', () => {
  it('moves a shared child to the parent whose loadNode settles last', async () => {
    const equal = fixture([
      { name: 'first', children: [2] },
      { name: 'second', children: [2] },
      { name: 'shared', mesh: 0 },
    ], [0, 1]);
    const deeperFirst = fixture([
      { name: 'first', children: [3] },
      { name: 'second', children: [2] },
      { name: 'shared', mesh: 0 },
      { name: 'middle', children: [2] },
    ], [0, 1]);
    const deeperSecond = fixture([
      { name: 'first', children: [2] },
      { name: 'second', children: [3] },
      { name: 'shared', mesh: 0 },
      { name: 'middle', children: [2] },
    ], [0, 1]);
    const meshFirst = fixture([
      { name: 'first', mesh: 0, children: [2] },
      { name: 'second', children: [2] },
      { name: 'shared', mesh: 0 },
    ], [0, 1]);
    const meshSecond = fixture([
      { name: 'first', children: [2] },
      { name: 'second', mesh: 0, children: [2] },
      { name: 'shared', mesh: 0 },
    ], [0, 1]);
    const delayedFirst = fixture([
      { name: 'first', mesh: 0, children: [2] },
      { name: 'second', children: [2] },
      { name: 'shared' },
    ], [0, 1]);
    const extraBranchFirst = fixture([
      { name: 'first', children: [2, 3] },
      { name: 'second', children: [2] },
      { name: 'shared', mesh: 0 },
      { name: 'slow', children: [4] },
      { name: 'tail' },
    ], [0, 1]);
    const longBranchFirst = fixture([
      { name: 'first', children: [2, 3] },
      { name: 'second', children: [2] },
      { name: 'shared', mesh: 0 },
      { name: 'branch0', children: [4] },
      { name: 'branch1', children: [5] },
      { name: 'branch2', children: [6] },
      { name: 'branch3', children: [7] },
      { name: 'branch4' },
    ], [0, 1]);
    const longPlainBranchFirst = fixture([
      { name: 'first', children: [2, 3] },
      { name: 'second', children: [2] },
      { name: 'shared' },
      { name: 'branch0', children: [4] },
      { name: 'branch1', children: [5] },
      { name: 'branch2', children: [6] },
      { name: 'branch3', children: [7] },
      { name: 'branch4' },
      { name: 'geometry', mesh: 0 },
    ], [0, 1, 8]);
    const cases = [
      [equal, 'second'], [deeperFirst, 'second'], [deeperSecond, 'middle'],
      [meshFirst, 'second'], [meshSecond, 'second'], [delayedFirst, 'first'],
      [extraBranchFirst, 'second'], [longBranchFirst, 'second'],
      [longPlainBranchFirst, 'first'],
    ] as const;
    if (process.env.PM_WASM_REQUIRED === '1') expect(existsSync(MODULE)).toBe(true);
    const wasm = process.env.PM_WASM_REQUIRED === '1' && existsSync(MODULE)
      ? await WebAssembly.compile(readFileSync(MODULE)) : null;
    for (const [bytes, expected] of cases) {
      expect(await parentOf(bytes)).toBe(expected);
      if (!wasm) continue;
      const g = globalThis as any;
      const hadProgressEvent = 'ProgressEvent' in g;
      if (!hadProgressEvent) g.ProgressEvent = class extends Event { constructor(type: string, init: any = {}) { super(type); Object.assign(this, init); } };
      try {
        const data = new PmWasm(new WebAssembly.Instance(wasm, imports)).importModel(bytes);
        const adapted = await object3dFromEngine(data, bytes);
        expect(adapted.object3d.getObjectByName('shared')?.parent?.name).toBe(expected);
        disposeModel(adapted.object3d);
      } finally {
        if (!hadProgressEvent) delete g.ProgressEvent;
      }
    }
  });

  it('agrees with the loader on generated cached-mesh and branching graphs', async () => {
    if (process.env.PM_WASM_REQUIRED !== '1') return;
    expect(existsSync(MODULE)).toBe(true);
    const wasm = await WebAssembly.compile(readFileSync(MODULE));
    for (const encoding of ['data', 'sparse', 'glb'] as const) {
      for (let seed = 1; seed <= 64; seed++) {
        const source = generated(seed);
        const bytes = encoding === 'sparse' ? sparseFixture(source) : encoding === 'glb' ? glbFixture(source) : source;
        const expected = await parentOf(bytes);
        const data = new PmWasm(new WebAssembly.Instance(wasm, imports)).importModel(bytes);
        const adapted = await object3dFromEngine(data, bytes);
        expect(adapted.object3d.getObjectByName('shared')?.parent?.name ?? null,
          `${encoding} seed ${seed}`).toBe(expected);
        disposeModel(adapted.object3d);
      }
    }
  });

  it('does not retain a default-scene child moved to another scene', async () => {
    const bytes = multiSceneFixture();
    expect(await parentOf(bytes)).toBe(null);
    if (process.env.PM_WASM_REQUIRED !== '1') return;
    const wasm = await WebAssembly.compile(readFileSync(MODULE));
    const data = new PmWasm(new WebAssembly.Instance(wasm, imports)).importModel(bytes);
    const adapted = await object3dFromEngine(data, bytes);
    expect(adapted.object3d.getObjectByName('shared')).toBeUndefined();
    disposeModel(adapted.object3d);
  });

  it('follows instancing dependencies and sorted primitive cache keys', async () => {
    if (process.env.PM_WASM_REQUIRED !== '1') return;
    const wasm = await WebAssembly.compile(readFileSync(MODULE));
    for (const bytes of [instancedFixture(), sharedPrimitiveFixture()]) {
      const expected = await parentOf(bytes);
      const data = new PmWasm(new WebAssembly.Instance(wasm, imports)).importModel(bytes);
      const adapted = await object3dFromEngine(data, bytes);
      expect(adapted.object3d.getObjectByName('shared')?.parent?.name ?? null).toBe(expected);
      disposeModel(adapted.object3d);
    }
  });

  it('follows material and off-scene animation dependency starts', async () => {
    if (process.env.PM_WASM_REQUIRED !== '1') return;
    const wasm = await WebAssembly.compile(readFileSync(MODULE));
    for (const source of [materialFixture('standard'), materialFixture('physical'),
      materialFixture('unlit'), animatedOffSceneFixture()]) {
      for (const bytes of [source, glbFixture(source)]) {
        const expected = await parentOf(bytes);
        const data = new PmWasm(new WebAssembly.Instance(wasm, imports)).importModel(bytes);
        const adapted = await object3dFromEngine(data, bytes);
        expect(adapted.object3d.getObjectByName('shared')?.parent?.name ?? null).toBe(expected);
        disposeModel(adapted.object3d);
      }
    }
  });

  it('orders embedded decoded images after competing data-URI buffers and reuses texture/source caches', async () => {
    const required = process.env.PM_WASM_REQUIRED === '1';
    if (required) expect(existsSync(MODULE)).toBe(true);
    const wasm = required ? await WebAssembly.compile(readFileSync(MODULE)) : null;
    for (const cache of ['single', 'sameSampler', 'differentSampler'] as const) {
      const bytes = embeddedImageFixture(cache);
      const expected = await withDecodedImages(() => parentOf(bytes));
      expect(expected, cache).toBe('first');
      if (!wasm) continue;
      const data = new PmWasm(new WebAssembly.Instance(wasm, imports)).importModel(bytes);
      const adapted = await withDecodedImages(() => object3dFromEngine(data, bytes));
      expect(adapted.object3d.getObjectByName('shared')?.parent?.name ?? null, cache).toBe(expected);
      disposeModel(adapted.object3d);
    }
  });
});
