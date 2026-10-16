/**
 * The three.js r180 reference for model import (pm-geom3d `gltf`): loads
 * each model with the app's own `loadGltf` (core/scene3d/import-model.ts)
 * and records what it built: the import facts (vertices, triangles, the
 * normalization's center and radius), every object in traversal order
 * (type, name, local matrix, geometry counts and bounds, material
 * parameters), and the animation clips.
 *
 *   bun engine/crates/pm-geom3d/tools/three-models.ts FILE... > reference.json
 *   bun engine/crates/pm-geom3d/tools/three-models.ts --corpus [DIR]
 *
 * `--corpus` writes the model corpus into DIR (default
 * ~/Library/Caches/powermove-engine/model-corpus, outside the repository):
 * the parity corpus's GLBs (engine/corpus/src/glb.ts) and synthetic edge
 * cases (strips and fans, matrix nodes, shared and mirrored meshes, sparse
 * quantized attributes, a skin with 8-bit weights, a node with a mesh, a
 * camera, and a light), then writes the reference to
 * tests/fixtures/three-models.json with each file's SHA-256.
 *
 * Runs under Bun without a DOM: images are stubbed (1 × 1 bitmaps), which
 * three.js only needs for texture upload; geometry, materials, and clips are
 * what GLTFLoader builds from the file.
 */
import { readFileSync } from 'node:fs';
import path from 'node:path';

const g = globalThis as any;
g.self ??= globalThis;
g.window ??= globalThis;
g.createImageBitmap ??= async () => ({ width: 1, height: 1, close() {} });
g.ProgressEvent ??= class ProgressEvent extends Event {
  constructor(type: string, init: Record<string, unknown> = {}) { super(type); Object.assign(this, init); }
};

const root = path.resolve(import.meta.dir, '../../../..');
const { loadGltf } = await import(path.join(root, 'apps/desktop/src/renderer/src/core/scene3d/import-model.ts'));

const round = (v: number) => (Number.isFinite(v) ? Math.fround(v) : v);

function material(m: any): unknown {
  if (!m) return null;
  const out: Record<string, unknown> = { type: m.type, name: m.name, side: m.side, transparent: m.transparent, opacity: round(m.opacity), alphaTest: round(m.alphaTest), vertexColors: m.vertexColors, flatShading: !!m.flatShading };
  for (const key of ['color', 'emissive', 'sheenColor', 'specularColor', 'attenuationColor']) if (m[key]?.isColor) out[key] = m[key].toArray().map(round);
  for (const key of ['roughness', 'metalness', 'emissiveIntensity', 'clearcoat', 'clearcoatRoughness', 'ior', 'transmission', 'thickness', 'attenuationDistance', 'iridescence', 'sheen', 'sheenRoughness', 'specularIntensity', 'anisotropy', 'dispersion', 'bumpScale', 'aoMapIntensity']) if (typeof m[key] === 'number') out[key] = round(m[key]);
  if (m.normalScale) out.normalScale = m.normalScale.toArray().map(round);
  const maps: Record<string, unknown> = {};
  for (const key of Object.keys(m)) {
    const t = m[key];
    if (t?.isTexture) maps[key] = { channel: t.channel, colorSpace: t.colorSpace, flipY: t.flipY, wrapS: t.wrapS, wrapT: t.wrapT, offset: t.offset.toArray().map(round), repeat: t.repeat.toArray().map(round), rotation: round(t.rotation) };
  }
  out.maps = maps;
  return out;
}

function geometry(o: any): unknown {
  const geo = o.geometry;
  if (!geo) return null;
  const attrs: Record<string, unknown> = {};
  for (const [name, a] of Object.entries<any>(geo.attributes)) {
    let sum = 0;
    for (let i = 0; i < a.count; i++) for (let c = 0; c < a.itemSize; c++) sum += a.getComponent(i, c);
    attrs[name] = { count: a.count, itemSize: a.itemSize, normalized: a.normalized, sum: round(sum) };
  }
  geo.computeBoundingBox();
  const b = geo.boundingBox;
  return {
    attributes: attrs,
    index: geo.index ? { count: geo.index.count, sum: Array.from(geo.index.array as ArrayLike<number>).reduce((s, v) => s + v, 0) } : null,
    groups: geo.groups.length,
    morphTargets: Object.fromEntries(Object.entries<any>(geo.morphAttributes).map(([k, v]) => [k, v.length])),
    bounds: [b.min.toArray().map(round), b.max.toArray().map(round)]
  };
}

const b64 = (bytes: Uint8Array) => Buffer.from(bytes).toString('base64');
const f32 = (v: number[]) => new Uint8Array(new Float32Array(v).buffer);

/** A self-contained glTF from parts: each buffer view's bytes go in one data URI. */
function gltf(parts: Uint8Array[], json: Record<string, unknown>, strides: Record<number, number> = {}): string {
  let offset = 0;
  let views_count = 0;
  const views = parts.map((p) => {
    const pad = (4 - (offset % 4)) % 4;
    offset += pad;
    const view: Record<string, number> = { buffer: 0, byteOffset: offset, byteLength: p.length };
    const stride = strides[views_count++];
    if (stride) view.byteStride = stride;
    offset += p.length;
    return view;
  });
  const bin = new Uint8Array(offset);
  let at = 0;
  parts.forEach((p, i) => { at = views[i]!.byteOffset; bin.set(p, at); });
  return JSON.stringify({ asset: { version: '2.0' }, buffers: [{ byteLength: bin.length, uri: `data:application/octet-stream;base64,${b64(bin)}` }], bufferViews: views, ...json });
}

function edgeCases(): Record<string, string> {
  const quad = [0, 0, 0, 1, 0, 0, 1, 1, 0, 0, 1, 0];
  const out: Record<string, string> = {};
  // Strips and fans in one mesh, under a node with a matrix.
  out['strip-fan.gltf'] = gltf([f32(quad), f32([0, 0, 0, 1, 0, 0, 0, 1, 0, 1, 1, 0, 0, 2, 0])], {
    accessors: [
      { bufferView: 0, componentType: 5126, count: 4, type: 'VEC3', min: [0, 0, 0], max: [1, 1, 0] },
      { bufferView: 1, componentType: 5126, count: 5, type: 'VEC3', min: [0, 0, 0], max: [1, 2, 0] }
    ],
    meshes: [{ name: 'Strips and fans', primitives: [{ attributes: { POSITION: 1 }, mode: 5 }, { attributes: { POSITION: 0 }, mode: 6 }] }],
    nodes: [{ name: 'Matrix node', mesh: 0, matrix: [0, 0, -2, 0, 0, 2, 0, 0, 2, 0, 0, 0, 3, -1, 0.5, 1] }],
    scenes: [{ name: 'Edge', nodes: [0] }], scene: 0
  });
  // One mesh on two nodes, one mirrored; an unnamed parent.
  out['shared-mesh.gltf'] = gltf([f32(quad), new Uint8Array(new Uint16Array([0, 1, 2, 0, 2, 3]).buffer)], {
    accessors: [
      { bufferView: 0, componentType: 5126, count: 4, type: 'VEC3', min: [0, 0, 0], max: [1, 1, 0] },
      { bufferView: 1, componentType: 5123, count: 6, type: 'SCALAR' }
    ],
    meshes: [{ name: 'Tile', primitives: [{ attributes: { POSITION: 0 }, indices: 1 }] }],
    nodes: [{ children: [1, 2], translation: [0, 0, -4] }, { name: 'Left', mesh: 0, translation: [-2, 0, 0] }, { name: 'Right', mesh: 0, scale: [-1, 1, 1], rotation: [0, 0.3826834, 0, 0.9238795] }],
    scenes: [{ nodes: [0] }]
  });
  // Quantized positions (u16 normalized) with a sparse override, i8 normals.
  const qpos = new Uint16Array([0, 0, 0, 0, 65535, 0, 0, 0, 65535, 0, 0, 0, 65535, 65535, 65535, 0]);
  const qnrm = new Int8Array([0, 0, 127, 0, 0, 0, 127, 0, 0, 0, 127, 0, 0, 0, 127, 0]);
  out['sparse-quantized.gltf'] = gltf([new Uint8Array(qpos.buffer), new Uint8Array(qnrm.buffer), new Uint8Array(new Uint8Array([2]).buffer), new Uint8Array(new Uint16Array([32768, 65535, 16384, 0]).buffer)], {
    extensionsUsed: ['KHR_mesh_quantization'], extensionsRequired: ['KHR_mesh_quantization'],
    accessors: [
      { bufferView: 0, byteOffset: 0, componentType: 5123, normalized: true, count: 4, type: 'VEC3', min: [0, 0, 0], max: [65535, 65535, 0],
        sparse: { count: 1, indices: { bufferView: 2, componentType: 5121 }, values: { bufferView: 3 } } },
      { bufferView: 1, componentType: 5120, normalized: true, count: 4, type: 'VEC3' }
    ],
    meshes: [{ primitives: [{ attributes: { POSITION: 0, NORMAL: 1 } }] }],
    nodes: [{ name: 'Quantized', mesh: 0, scale: [2, 2, 2] }],
    scenes: [{ nodes: [0] }]
  }, { 0: 8, 1: 4 });
  // A skin with u8 normalized weights, joints in a chain, the skinned node moved.
  const pos = [0, 0, 0, 1, 0, 0, 1, 2, 0, 0, 2, 0, 0, 4, 0, 1, 4, 0];
  const joints = new Uint8Array([0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0]);
  const weights = new Uint8Array([255, 0, 0, 0, 200, 50, 0, 0, 100, 100, 0, 0, 100, 100, 0, 0, 0, 255, 0, 0, 10, 240, 0, 0]);
  const ibm = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, -2, 0, 1];
  out['skinned-u8.gltf'] = gltf([f32(pos), joints, weights, f32(ibm), new Uint8Array(new Uint16Array([0, 1, 2, 0, 2, 3, 3, 2, 5, 3, 5, 4]).buffer)], {
    accessors: [
      { bufferView: 0, componentType: 5126, count: 6, type: 'VEC3', min: [0, 0, 0], max: [1, 4, 0] },
      { bufferView: 1, componentType: 5121, count: 6, type: 'VEC4' },
      { bufferView: 2, componentType: 5121, normalized: true, count: 6, type: 'VEC4' },
      { bufferView: 3, componentType: 5126, count: 2, type: 'MAT4' },
      { bufferView: 4, componentType: 5123, count: 12, type: 'SCALAR' }
    ],
    meshes: [{ name: 'Bend', primitives: [{ attributes: { POSITION: 0, JOINTS_0: 1, WEIGHTS_0: 2 }, indices: 4 }] }],
    skins: [{ joints: [1, 2], inverseBindMatrices: 3 }],
    nodes: [
      { name: 'Skinned', mesh: 0, skin: 0, translation: [0.5, 0, 0] },
      { name: 'Hip', children: [2], rotation: [0, 0, 0.0871557, 0.9961947] },
      { name: 'Knee', translation: [0, 2, 0], rotation: [0, 0, 0.258819, 0.9659258] }
    ],
    scenes: [{ nodes: [0, 1] }]
  });
  // A node with a mesh, a camera, and a light; nested children.
  out['camera-light.gltf'] = gltf([f32(quad), new Uint8Array(new Uint16Array([0, 1, 2, 0, 2, 3]).buffer)], {
    extensionsUsed: ['KHR_lights_punctual'],
    extensions: { KHR_lights_punctual: { lights: [{ type: 'directional', color: [1, 0.9, 0.8], intensity: 3 }, { name: 'Lamp', type: 'point', intensity: 20, range: 9 }] } },
    accessors: [
      { bufferView: 0, componentType: 5126, count: 4, type: 'VEC3', min: [0, 0, 0], max: [1, 1, 0] },
      { bufferView: 1, componentType: 5123, count: 6, type: 'SCALAR' }
    ],
    cameras: [{ type: 'perspective', perspective: { yfov: 0.7, znear: 0.1, zfar: 100 } }],
    meshes: [{ name: 'Panel', primitives: [{ attributes: { POSITION: 0 }, indices: 1 }] }],
    nodes: [
      { name: 'Rig', mesh: 0, camera: 0, extensions: { KHR_lights_punctual: { light: 0 } }, children: [1], translation: [0, 1, 0] },
      { name: 'Lamp node', extensions: { KHR_lights_punctual: { light: 1 } }, translation: [2, 2, 2], children: [2] },
      { name: 'Panel copy', mesh: 0, translation: [0, 0, 3] }
    ],
    scenes: [{ nodes: [0] }]
  });
  return out;
}

const args = process.argv.slice(2);
let files: string[] = args;
let referencePath: string | null = null;
if (args[0] === '--corpus') {
  const { mkdirSync, writeFileSync } = await import('node:fs');
  const dir = args[1] ?? path.join(process.env.HOME!, 'Library/Caches/powermove-engine/model-corpus');
  mkdirSync(dir, { recursive: true });
  const { corpusGlb, corpusMaterialsGlb, corpusObj } = await import(path.join(root, 'engine/corpus/src/glb.ts'));
  writeFileSync(path.join(dir, 'corpus-model.glb'), corpusGlb());
  writeFileSync(path.join(dir, 'corpus-materials.glb'), corpusMaterialsGlb());
  writeFileSync(path.join(dir, 'corpus-gem.obj'), corpusObj());
  for (const [name, text] of Object.entries(edgeCases())) writeFileSync(path.join(dir, name), text);
  files = ['corpus-model.glb', 'corpus-materials.glb', ...Object.keys(edgeCases())].map((n) => path.join(dir, n));
  referencePath = path.join(import.meta.dir, '../tests/fixtures/three-models.json');
}

const { createHash } = await import('node:crypto');
const out: Record<string, unknown> = {};
for (const file of files) {
  const bytes = readFileSync(file);
  try {
    const { object3d, animations, vertices, triangles } = await loadGltf(new Blob([bytes]));
    const normalization = object3d.children[0];
    const scene = normalization.children[0];
    const objects: unknown[] = [];
    scene.traverse((o: any) => {
      objects.push({
        type: o.type,
        name: o.name,
        matrix: o.matrix.toArray().map(round),
        count: o.isInstancedMesh ? o.count : undefined,
        geometry: geometry(o),
        material: Array.isArray(o.material) ? o.material.map(material) : material(o.material),
        skin: o.isSkinnedMesh ? { bones: o.skeleton.bones.map((b: any) => b.name), bindMatrix: o.bindMatrix.toArray().map(round) } : undefined,
        light: o.isLight ? { type: o.type, color: o.color.toArray().map(round), intensity: round(o.intensity), distance: o.distance, angle: o.angle, penumbra: o.penumbra, decay: o.decay } : undefined
      });
    });
    out[path.basename(file)] = {
      sha256: createHash('sha256').update(bytes).digest('hex'),
      vertices,
      triangles,
      radius: 1 / normalization.scale.x,
      center: scene.position.toArray().map((v: number) => -v),
      objects,
      animations: animations.map((a: any) => ({ name: a.name, duration: round(a.duration), tracks: a.tracks.map((t: any) => ({ name: t.name, times: t.times.length, values: t.values.length, interpolation: t.getInterpolation() })) }))
    };
  } catch (error) {
    out[path.basename(file)] = { error: error instanceof Error ? error.message : String(error) };
  }
}
const text = JSON.stringify(out, null, 1) + '\n';
if (referencePath) {
  const { writeFileSync } = await import('node:fs');
  writeFileSync(referencePath, text);
  console.error(`wrote ${referencePath} (${files.length} models)`);
} else process.stdout.write(text);
