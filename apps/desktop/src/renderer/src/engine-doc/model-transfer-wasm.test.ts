import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { expect, it } from 'vitest';
import * as THREE from 'three';
import { object3dFromEngine } from '../core/scene3d/compat-object3d';
import { imports, PmWasm } from './wasm';

const MODULE = fileURLToPath(new URL('../../public/engine/pm_wasm.wasm', import.meta.url));

function triangle(): Uint8Array {
  const positions = new Float32Array([0, 0, 0, 1, 0, 0, 0, 1, 0]);
  const normals = new Float32Array([0, 0, 1, 0, 0, 1, 0, 0, 1]);
  const uv = new Float32Array([0, 0, 1, 0, 0, 1]);
  const indices = new Uint16Array([0, 1, 2]);
  const chunks = [positions, normals, uv, indices].map(a => Buffer.from(a.buffer));
  const buffer = Buffer.concat(chunks);
  const views = chunks.map((bytes, index) => ({
    buffer: 0,
    byteOffset: chunks.slice(0, index).reduce((n, chunk) => n + chunk.length, 0),
    byteLength: bytes.length,
  }));
  const gltf = {
    asset: { version: '2.0' },
    buffers: [{ byteLength: buffer.length, uri: `data:application/octet-stream;base64,${buffer.toString('base64')}` }],
    bufferViews: views,
    accessors: [
      { bufferView: 0, componentType: 5126, count: 3, type: 'VEC3', min: [0, 0, 0], max: [1, 1, 0] },
      { bufferView: 1, componentType: 5126, count: 3, type: 'VEC3' },
      { bufferView: 2, componentType: 5126, count: 3, type: 'VEC2' },
      { bufferView: 3, componentType: 5123, count: 3, type: 'SCALAR' },
    ],
    meshes: [{ name: 'Triangle', extras: { meshTag: 'same' }, primitives: [{
      attributes: { POSITION: 0, NORMAL: 1, TEXCOORD_0: 2 }, indices: 3,
      extras: { primitiveTag: 'same' },
    }] }],
    nodes: [{ name: 'Visible', mesh: 0, extras: { nodeTag: 'same' } }],
    scenes: [{ nodes: [0], extras: { sceneTag: 'same' } }],
    scene: 0,
  };
  return new TextEncoder().encode(JSON.stringify(gltf));
}

function firstMesh(root: THREE.Object3D): THREE.Mesh {
  let found: THREE.Mesh | null = null;
  root.traverse(object => { if (!found && (object as THREE.Mesh).isMesh) found = object as THREE.Mesh; });
  if (!found) throw new Error('Imported scene has no mesh');
  return found;
}

it('real WASM packed model equals JSON and owns typed views across memory growth and later calls', async () => {
  const module = await WebAssembly.compile(readFileSync(MODULE));
  const wasm = new PmWasm(new WebAssembly.Instance(module, imports));
  expect(wasm.x.pm_model_import_packed).toBeTypeOf('function');
  expect(wasm.x.pm_out_release).toBeTypeOf('function');
  const bytes = triangle();
  const baselineLive = wasm.heap().live;
  const json = wasm.importModel(bytes);
  const packed = wasm.importModel(bytes, true);
  const primitive = packed.meshes[0].primitives[0];
  const reference = json.meshes[0].primitives[0];
  expect(packed.metadata).toEqual(json.metadata);
  expect(packed.objects).toEqual(json.objects);
  expect(packed.facts).toEqual(json.facts);
  expect(primitive.positions).toBeInstanceOf(Float32Array);
  expect(primitive.normals).toBeInstanceOf(Float32Array);
  expect(primitive.uv).toBeInstanceOf(Float32Array);
  expect(primitive.indices).toBeInstanceOf(Uint32Array);
  expect(Array.from(primitive.positions)).toEqual(reference.positions.flat());
  expect(Array.from(primitive.normals)).toEqual(reference.normals.flat());
  expect(Array.from(primitive.uv)).toEqual(reference.uv.flat());
  expect(Array.from(primitive.indices)).toEqual(reference.indices);
  expect(primitive.positions.buffer).toBe(primitive.normals.buffer);
  expect(primitive.positions.buffer).toBe(primitive.indices.buffer);
  expect(primitive.positions.buffer).not.toBe(wasm.x.memory.buffer);

  const [fromJson, fromPacked] = await Promise.all([
    object3dFromEngine(json), object3dFromEngine(packed),
  ]);
  const oldMesh = firstMesh(fromJson.object3d);
  const newMesh = firstMesh(fromPacked.object3d);
  for (const name of ['position', 'normal', 'uv']) {
    expect(Array.from(newMesh.geometry.getAttribute(name).array)).toEqual(
      Array.from(oldMesh.geometry.getAttribute(name).array));
  }
  expect(Array.from(newMesh.geometry.index!.array)).toEqual(Array.from(oldMesh.geometry.index!.array));
  expect(newMesh.geometry.getAttribute('position').array).toBe(primitive.positions);
  expect(newMesh.userData).toEqual(oldMesh.userData);
  expect(fromPacked.vertices).toBe(fromJson.vertices);
  expect(fromPacked.triangles).toBe(fromJson.triangles);

  // The packed result was copied into JS-owned memory. Neither growing the
  // module nor replacing its input/output buffers can detach these views.
  const savedBuffer = primitive.positions.buffer;
  wasm.x.memory.grow(1);
  wasm.importModel(bytes); // overwrites the module's input and output
  expect(primitive.positions.buffer).toBe(savedBuffer);
  expect(savedBuffer.byteLength).toBeGreaterThan(0);
  expect(Array.from(primitive.positions)).toEqual(reference.positions.flat());
  expect(Array.from(primitive.indices)).toEqual(reference.indices);

  // Packed success explicitly releases both WASM buffers after the one copy.
  wasm.importModel(bytes, true);
  expect(wasm.x.pm_out_len()).toBe(0);
  expect(wasm.heap().live - baselineLive).toBeLessThan(64 * 1024);
  expect(wasm.setModel('triangle', 'gltf', bytes)).toBeNull();
  expect(Array.from(primitive.positions)).toEqual(reference.positions.flat());
  expect(Array.from(primitive.indices)).toEqual(reference.indices);
  wasm.forgetModel('triangle');
});
