/**
 * Small, deterministic binary glTF 2.0 files exercising the loader paths the
 * 3D corpus needs without Blender.
 *
 * - `corpusGlb`: an embedded PNG base-color texture with UVs, vertex colors,
 *   emissive strength, a two-joint skinned mesh, and two animation clips
 *   (three.js plays every clip at weight 1, looping).
 * - `corpusMaterialsGlb`: one primitive per extension three.js r180's
 *   GLTFLoader honors without an external decoder (the app configures no
 *   Draco, meshopt, or KTX2 decoder), plus alpha modes, normal, occlusion
 *   (second UV set) and bump maps, a morph target with a weights clip, point
 *   and line primitives, a primitive without normals, quantized attributes,
 *   GPU instancing, and embedded punctual lights.
 */
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fixturesDir } from './catalog';
import { encodePng } from './png';

type Accessor = { bufferView: number; componentType: number; count: number; type: string; min?: number[]; max?: number[]; normalized?: boolean };

class GltfWriter {
  private chunks: Uint8Array[] = [];
  private length = 0;
  bufferViews: Array<{ buffer: 0; byteOffset: number; byteLength: number; target?: number }> = [];
  accessors: Accessor[] = [];

  private view(bytes: Uint8Array, target?: number): number {
    const padding = (4 - (this.length % 4)) % 4;
    if (padding) { this.chunks.push(new Uint8Array(padding)); this.length += padding; }
    this.bufferViews.push({ buffer: 0, byteOffset: this.length, byteLength: bytes.length, ...(target ? { target } : {}) });
    this.chunks.push(bytes); this.length += bytes.length;
    return this.bufferViews.length - 1;
  }

  floats(values: number[], type: 'SCALAR' | 'VEC2' | 'VEC3' | 'VEC4' | 'MAT4', options: { minMax?: boolean; target?: number } = {}): number {
    const width = { SCALAR: 1, VEC2: 2, VEC3: 3, VEC4: 4, MAT4: 16 }[type];
    const data = new Float32Array(values);
    const accessor: Accessor = { bufferView: this.view(new Uint8Array(data.buffer), options.target), componentType: 5126, count: values.length / width, type };
    if (options.minMax) {
      accessor.min = Array.from({ length: width }, (_, c) => Math.min(...values.filter((_, i) => i % width === c).map(v => Math.fround(v))));
      accessor.max = Array.from({ length: width }, (_, c) => Math.max(...values.filter((_, i) => i % width === c).map(v => Math.fround(v))));
    }
    this.accessors.push(accessor);
    return this.accessors.length - 1;
  }

  /** Integer-typed attribute data (KHR_mesh_quantization, instancing, colors). */
  typed(values: number[], componentType: 5120 | 5121 | 5122 | 5123, type: 'SCALAR' | 'VEC2' | 'VEC3' | 'VEC4', options: { normalized?: boolean; minMax?: boolean; target?: number } = {}): number {
    const width = { SCALAR: 1, VEC2: 2, VEC3: 3, VEC4: 4 }[type];
    const Array_ = { 5120: Int8Array, 5121: Uint8Array, 5122: Int16Array, 5123: Uint16Array }[componentType];
    const data = new Array_(values);
    // Vertex attributes need each element aligned to 4 bytes (glTF 2.0 3.6.2.4).
    const element = width * data.BYTES_PER_ELEMENT, stride = Math.ceil(element / 4) * 4;
    const bytes = new Uint8Array((values.length / width) * stride);
    for (let i = 0; i < values.length / width; i++) bytes.set(new Uint8Array(data.buffer, i * element, element), i * stride);
    const view = this.view(bytes, options.target);
    if (stride !== element) (this.bufferViews[view] as any).byteStride = stride;
    const accessor: Accessor = { bufferView: view, componentType, count: values.length / width, type, ...(options.normalized ? { normalized: true } : {}) };
    if (options.minMax) {
      accessor.min = Array.from({ length: width }, (_, c) => Math.min(...values.filter((_, i) => i % width === c)));
      accessor.max = Array.from({ length: width }, (_, c) => Math.max(...values.filter((_, i) => i % width === c)));
    }
    this.accessors.push(accessor);
    return this.accessors.length - 1;
  }

  indices(values: number[]): number {
    const data = new Uint16Array(values);
    this.accessors.push({ bufferView: this.view(new Uint8Array(data.buffer), 34963), componentType: 5123, count: values.length, type: 'SCALAR' });
    return this.accessors.length - 1;
  }

  joints(values: number[]): number {
    this.accessors.push({ bufferView: this.view(new Uint8Array(values), 34962), componentType: 5121, count: values.length / 4, type: 'VEC4' });
    return this.accessors.length - 1;
  }

  image(bytes: Uint8Array): number { return this.view(bytes); }

  binary(): Uint8Array {
    const padding = (4 - (this.length % 4)) % 4;
    const out = new Uint8Array(this.length + padding);
    let offset = 0;
    for (const part of this.chunks) { out.set(part, offset); offset += part.length; }
    return out;
  }
}

function box(w: number, h: number, d: number) {
  const x = w / 2, y = h / 2, z = d / 2;
  const faces: Array<{ n: [number, number, number]; c: Array<[number, number, number]> }> = [
    { n: [0, 0, 1], c: [[-x, -y, z], [x, -y, z], [x, y, z], [-x, y, z]] },
    { n: [0, 0, -1], c: [[x, -y, -z], [-x, -y, -z], [-x, y, -z], [x, y, -z]] },
    { n: [1, 0, 0], c: [[x, -y, z], [x, -y, -z], [x, y, -z], [x, y, z]] },
    { n: [-1, 0, 0], c: [[-x, -y, -z], [-x, -y, z], [-x, y, z], [-x, y, -z]] },
    { n: [0, 1, 0], c: [[-x, y, z], [x, y, z], [x, y, -z], [-x, y, -z]] },
    { n: [0, -1, 0], c: [[-x, -y, -z], [x, -y, -z], [x, -y, z], [-x, -y, z]] },
  ];
  const positions: number[] = [], normals: number[] = [], uvs: number[] = [], indices: number[] = [];
  faces.forEach((face, f) => {
    face.c.forEach(corner => positions.push(...corner));
    for (let i = 0; i < 4; i++) normals.push(...face.n);
    uvs.push(0, 1, 1, 1, 1, 0, 0, 0);
    indices.push(f * 4, f * 4 + 1, f * 4 + 2, f * 4, f * 4 + 2, f * 4 + 3);
  });
  return { positions, normals, uvs, indices };
}

function octahedron(r: number) {
  const v = [[r, 0, 0], [-r, 0, 0], [0, r, 0], [0, -r, 0], [0, 0, r], [0, 0, -r]];
  const faces = [[4, 0, 2], [0, 5, 2], [5, 1, 2], [1, 4, 2], [0, 4, 3], [5, 0, 3], [1, 5, 3], [4, 1, 3]];
  const palette = [[1, .35, .1, 1], [.1, .55, 1, 1], [.2, .9, .4, 1], [1, .85, .2, 1]];
  const positions: number[] = [], normals: number[] = [], colors: number[] = [];
  faces.forEach((face, index) => {
    const [a, b, c] = face.map(i => v[i]!) as [number[], number[], number[]];
    const u = [b[0]! - a[0]!, b[1]! - a[1]!, b[2]! - a[2]!], w = [c[0]! - a[0]!, c[1]! - a[1]!, c[2]! - a[2]!];
    const n = [u[1]! * w[2]! - u[2]! * w[1]!, u[2]! * w[0]! - u[0]! * w[2]!, u[0]! * w[1]! - u[1]! * w[0]!];
    const l = Math.hypot(n[0]!, n[1]!, n[2]!);
    for (const corner of [a, b, c]) { positions.push(...corner); normals.push(n[0]! / l, n[1]! / l, n[2]! / l); colors.push(...palette[index % 4]!); }
  });
  return { positions, normals, colors };
}

/** A square column along +Y in 4 segments, weighted between two joints. */
function strip(width: number, height: number) {
  const rings = 5, s = width / 2;
  const corners: Array<[number, number]> = [[-s, s], [s, s], [s, -s], [-s, -s]];
  const positions: number[] = [], normals: number[] = [], joints: number[] = [], weights: number[] = [], indices: number[] = [];
  // Four flat sides, each with its own vertices so normals stay flat.
  for (let side = 0; side < 4; side++) {
    const [x0, z0] = corners[side]!, [x1, z1] = corners[(side + 1) % 4]!;
    const nx = (z1 - z0), nz = -(x1 - x0), nl = Math.hypot(nx, nz);
    const base = positions.length / 3;
    for (let r = 0; r < rings; r++) {
      const t = r / (rings - 1), y = t * height;
      for (const [x, z] of [[x0, z0], [x1, z1]]) {
        positions.push(x!, y, z!); normals.push(nx / nl, 0, nz / nl);
        joints.push(0, 1, 0, 0); weights.push(1 - t, t, 0, 0);
      }
    }
    for (let r = 0; r < rings - 1; r++) {
      const a = base + r * 2;
      indices.push(a, a + 1, a + 3, a, a + 3, a + 2);
    }
  }
  return { positions, normals, joints, weights, indices };
}

function quaternionY(degrees: number): number[] { const h = degrees * Math.PI / 360; return [0, Math.sin(h), 0, Math.cos(h)]; }
function quaternionZ(degrees: number): number[] { const h = degrees * Math.PI / 360; return [0, 0, Math.sin(h), Math.cos(h)]; }

function checker(): Uint8Array {
  const size = 8, data = new Uint8Array(size * size * 4);
  for (let y = 0; y < size; y++) for (let x = 0; x < size; x++) {
    const i = (y * size + x) * 4, on = (x + y) % 2 === 0;
    data.set(on ? [240, 236, 220, 255] : [36, 92, 200, 255], i);
  }
  return encodePng({ width: size, height: size, channels: 4, data });
}

export function corpusGlb(): Uint8Array {
  const g = new GltfWriter();
  const body = box(1, 1, 1);
  const bodyAttributes = {
    POSITION: g.floats(body.positions, 'VEC3', { minMax: true, target: 34962 }),
    NORMAL: g.floats(body.normals, 'VEC3', { target: 34962 }),
    TEXCOORD_0: g.floats(body.uvs, 'VEC2', { target: 34962 }),
  };
  const bodyIndices = g.indices(body.indices);
  const gem = octahedron(.45);
  const gemAttributes = {
    POSITION: g.floats(gem.positions, 'VEC3', { minMax: true, target: 34962 }),
    NORMAL: g.floats(gem.normals, 'VEC3', { target: 34962 }),
    COLOR_0: g.floats(gem.colors, 'VEC4', { target: 34962 }),
  };
  const column = strip(.3, 1.2);
  const columnAttributes = {
    POSITION: g.floats(column.positions, 'VEC3', { minMax: true, target: 34962 }),
    NORMAL: g.floats(column.normals, 'VEC3', { target: 34962 }),
    JOINTS_0: g.joints(column.joints),
    WEIGHTS_0: g.floats(column.weights, 'VEC4', { target: 34962 }),
  };
  const columnIndices = g.indices(column.indices);
  const inverseBind = g.floats([
    1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1,
    1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, -.6, 0, 1,
  ], 'MAT4');
  const image = g.image(checker());
  const spinTimes = g.floats([0, .5, 1, 1.5, 2], 'SCALAR', { minMax: true });
  const spinValues = g.floats([0, 90, 180, 270, 360].flatMap(quaternionY), 'VEC4');
  const bendTimes = g.floats([0, 1, 2], 'SCALAR', { minMax: true });
  const bendValues = g.floats([0, 40, 0].flatMap(quaternionZ), 'VEC4');
  const bobTimes = g.floats([0, .75, 1.5], 'SCALAR', { minMax: true });
  const bobValues = g.floats([1.1, .2, 0, 1.1, .6, 0, 1.1, .2, 0], 'VEC3');
  const bin = g.binary();
  const json = {
    asset: { version: '2.0', generator: 'Powermove engine corpus generator' },
    extensionsUsed: ['KHR_materials_emissive_strength'],
    scene: 0,
    scenes: [{ name: 'Corpus', nodes: [0, 1, 2, 3] }],
    nodes: [
      { name: 'Body', mesh: 0 },
      { name: 'Gem', mesh: 1, translation: [1.1, .2, 0] },
      { name: 'Column', mesh: 2, skin: 0 },
      { name: 'Root joint', translation: [-1.1, -.6, 0], children: [4] },
      { name: 'Bend joint', translation: [0, .6, 0] },
    ],
    meshes: [
      { name: 'Body', primitives: [{ attributes: bodyAttributes, indices: bodyIndices, material: 0 }] },
      { name: 'Gem', primitives: [{ attributes: gemAttributes, material: 1 }] },
      { name: 'Column', primitives: [{ attributes: columnAttributes, indices: columnIndices, material: 2 }] },
    ],
    skins: [{ joints: [3, 4], inverseBindMatrices: inverseBind, skeleton: 3 }],
    materials: [
      { name: 'Checker', pbrMetallicRoughness: { baseColorTexture: { index: 0 }, metallicFactor: .1, roughnessFactor: .6 } },
      { name: 'Gem', pbrMetallicRoughness: { baseColorFactor: [1, 1, 1, 1], metallicFactor: 0, roughnessFactor: .3 },
        emissiveFactor: [.2, .05, 0], extensions: { KHR_materials_emissive_strength: { emissiveStrength: 2 } } },
      { name: 'Column', pbrMetallicRoughness: { baseColorFactor: [.9, .3, .5, 1], metallicFactor: .5, roughnessFactor: .4 }, doubleSided: true },
    ],
    textures: [{ source: 0, sampler: 0 }],
    images: [{ bufferView: image, mimeType: 'image/png' }],
    samplers: [{ magFilter: 9728, minFilter: 9728, wrapS: 10497, wrapT: 10497 }],
    animations: [
      { name: 'Spin and bend', samplers: [{ input: spinTimes, output: spinValues, interpolation: 'LINEAR' }, { input: bendTimes, output: bendValues, interpolation: 'LINEAR' }],
        channels: [{ sampler: 0, target: { node: 0, path: 'rotation' } }, { sampler: 1, target: { node: 4, path: 'rotation' } }] },
      { name: 'Bob', samplers: [{ input: bobTimes, output: bobValues, interpolation: 'LINEAR' }], channels: [{ sampler: 0, target: { node: 1, path: 'translation' } }] },
    ],
    accessors: g.accessors,
    bufferViews: g.bufferViews,
    buffers: [{ byteLength: bin.length }],
  };
  return packGlb(json, bin);
}

function packGlb(json: unknown, bin: Uint8Array): Uint8Array {
  let text = new TextEncoder().encode(JSON.stringify(json));
  const textPadding = (4 - (text.length % 4)) % 4;
  if (textPadding) { const padded = new Uint8Array(text.length + textPadding).fill(0x20); padded.set(text); text = padded; }
  const total = 12 + 8 + text.length + 8 + bin.length;
  const out = new Uint8Array(total);
  const view = new DataView(out.buffer);
  view.setUint32(0, 0x46546c67, true); view.setUint32(4, 2, true); view.setUint32(8, total, true);
  view.setUint32(12, text.length, true); view.setUint32(16, 0x4e4f534a, true); out.set(text, 20);
  const binAt = 20 + text.length;
  view.setUint32(binAt, bin.length, true); view.setUint32(binAt + 4, 0x004e4942, true); out.set(bin, binAt + 8);
  return out;
}

/** A UV sphere with a second UV set (TEXCOORD_1, mirrored) for occlusion maps. */
function sphere(r: number, segments = 24, rings = 16) {
  const positions: number[] = [], normals: number[] = [], uvs: number[] = [], uvs1: number[] = [], indices: number[] = [];
  for (let y = 0; y <= rings; y++) {
    const v = y / rings, theta = v * Math.PI;
    for (let x = 0; x <= segments; x++) {
      const u = x / segments, phi = u * Math.PI * 2;
      const nx = -Math.cos(phi) * Math.sin(theta), ny = Math.cos(theta), nz = Math.sin(phi) * Math.sin(theta);
      positions.push(nx * r, ny * r, nz * r); normals.push(nx, ny, nz); uvs.push(u, v); uvs1.push(1 - u, v);
    }
  }
  for (let y = 0; y < rings; y++) for (let x = 0; x < segments; x++) {
    const a = y * (segments + 1) + x, b = a + segments + 1;
    if (y !== 0) indices.push(a, b, a + 1);
    if (y !== rings - 1) indices.push(a + 1, b, b + 1);
  }
  return { positions, normals, uvs, uvs1, indices };
}

/** A unit quad facing +Z. */
function quad(size: number) {
  const s = size / 2;
  return { positions: [-s, -s, 0, s, -s, 0, s, s, 0, -s, s, 0], normals: [0, 0, 1, 0, 0, 1, 0, 0, 1, 0, 0, 1], uvs: [0, 1, 1, 1, 1, 0, 0, 0], indices: [0, 1, 2, 0, 2, 3] };
}

function texturePng(size: number, pixel: (x: number, y: number) => [number, number, number, number]): Uint8Array {
  const data = new Uint8Array(size * size * 4);
  for (let y = 0; y < size; y++) for (let x = 0; x < size; x++) data.set(pixel(x, y), (y * size + x) * 4);
  return encodePng({ width: size, height: size, channels: 4, data });
}

export function corpusMaterialsGlb(): Uint8Array {
  const g = new GltfWriter();
  const ball = sphere(.35);
  const ballAttributes = (uv1 = false) => ({
    POSITION: g.floats(ball.positions, 'VEC3', { minMax: true, target: 34962 }),
    NORMAL: g.floats(ball.normals, 'VEC3', { target: 34962 }),
    TEXCOORD_0: g.floats(ball.uvs, 'VEC2', { target: 34962 }),
    ...(uv1 ? { TEXCOORD_1: g.floats(ball.uvs1, 'VEC2', { target: 34962 }) } : {}),
  });
  const ballIndices = g.indices(ball.indices);
  const plain = ballAttributes();
  const withUv1 = ballAttributes(true);
  const card = quad(.7);
  const cardAttributes = {
    POSITION: g.floats(card.positions, 'VEC3', { minMax: true, target: 34962 }),
    NORMAL: g.floats(card.normals, 'VEC3', { target: 34962 }),
    TEXCOORD_0: g.floats(card.uvs, 'VEC2', { target: 34962 }),
  };
  const cardIndices = g.indices(card.indices);

  // Textures: checker, alpha checker, normal bumps, bump height, and the WebP and AVIF fixtures.
  const checkerImage = g.image(texturePng(8, (x, y) => (x + y) % 2 ? [36, 92, 200, 255] : [240, 236, 220, 255]));
  const alphaImage = g.image(texturePng(8, (x, y) => (x + y) % 2 ? [255, 107, 26, 0] : [255, 107, 26, 255]));
  const normalImage = g.image(texturePng(16, (x, y) => {
    const nx = Math.sin(x / 16 * Math.PI * 4) * .5, ny = Math.cos(y / 16 * Math.PI * 4) * .5, nz = Math.sqrt(Math.max(0, 1 - nx * nx - ny * ny));
    return [Math.round((nx * .5 + .5) * 255), Math.round((ny * .5 + .5) * 255), Math.round((nz * .5 + .5) * 255), 255];
  }));
  const bumpImage = g.image(texturePng(16, (x, y) => { const h = Math.round(((Math.sin(x * .8) + Math.cos(y * .8)) * .25 + .5) * 255); return [h, h, h, 255]; }));
  const occlusionImage = g.image(texturePng(8, x => { const v = x < 4 ? 80 : 255; return [v, v, v, 255]; }));
  const webpImage = g.image(new Uint8Array(readFileSync(path.join(fixturesDir, 'still.webp'))));
  const avifImage = g.image(new Uint8Array(readFileSync(path.join(fixturesDir, 'still.avif'))));
  const images = [
    { bufferView: checkerImage, mimeType: 'image/png' }, { bufferView: alphaImage, mimeType: 'image/png' },
    { bufferView: normalImage, mimeType: 'image/png' }, { bufferView: bumpImage, mimeType: 'image/png' },
    { bufferView: occlusionImage, mimeType: 'image/png' }, { bufferView: webpImage, mimeType: 'image/webp' },
    { bufferView: avifImage, mimeType: 'image/avif' },
  ];
  const textures = [
    { source: 0, sampler: 0 }, { source: 1, sampler: 0 }, { source: 2, sampler: 1 }, { source: 3, sampler: 1 }, { source: 4, sampler: 1 },
    { sampler: 1, extensions: { EXT_texture_webp: { source: 5 } } }, { sampler: 1, extensions: { EXT_texture_avif: { source: 6 } } },
  ];
  const pbr = (color: number[], metallic = 0, roughness = .5) => ({ baseColorFactor: color, metallicFactor: metallic, roughnessFactor: roughness });

  // Materials, one extension or feature each.
  const materials = [
    { name: 'Unlit', pbrMetallicRoughness: pbr([1, .42, .1, 1]), extensions: { KHR_materials_unlit: {} } },
    { name: 'Clearcoat', pbrMetallicRoughness: pbr([.7, .05, .05, 1], .2, .6), extensions: { KHR_materials_clearcoat: { clearcoatFactor: 1, clearcoatRoughnessFactor: .08 } } },
    { name: 'Sheen', pbrMetallicRoughness: pbr([.25, .05, .3, 1], 0, .8), extensions: { KHR_materials_sheen: { sheenColorFactor: [1, .4, .8], sheenRoughnessFactor: .4 } } },
    { name: 'Iridescence', pbrMetallicRoughness: pbr([.1, .1, .12, 1], 1, .15), extensions: { KHR_materials_iridescence: { iridescenceFactor: 1, iridescenceIor: 1.3, iridescenceThicknessMinimum: 100, iridescenceThicknessMaximum: 400 } } },
    { name: 'Transmission, volume, IOR', pbrMetallicRoughness: pbr([.9, 1, .95, 1], 0, .05), extensions: {
      KHR_materials_transmission: { transmissionFactor: 1 }, KHR_materials_volume: { thicknessFactor: .4, attenuationDistance: .8, attenuationColor: [.4, .9, .6] }, KHR_materials_ior: { ior: 1.5 } } },
    { name: 'Specular, IOR', pbrMetallicRoughness: pbr([.2, .3, .8, 1], 0, .3), extensions: { KHR_materials_specular: { specularFactor: .6, specularColorFactor: [1, .5, .2] }, KHR_materials_ior: { ior: 1.8 } } },
    { name: 'Anisotropy', pbrMetallicRoughness: pbr([.85, .85, .9, 1], 1, .45), extensions: { KHR_materials_anisotropy: { anisotropyStrength: .8, anisotropyRotation: .6 } } },
    { name: 'Dispersion', pbrMetallicRoughness: pbr([1, 1, 1, 1], 0, 0), extensions: { KHR_materials_transmission: { transmissionFactor: 1 }, KHR_materials_volume: { thicknessFactor: .6 }, KHR_materials_ior: { ior: 1.6 }, KHR_materials_dispersion: { dispersion: 1 } } },
    { name: 'Texture transform', pbrMetallicRoughness: { ...pbr([1, 1, 1, 1]), baseColorTexture: { index: 0, extensions: { KHR_texture_transform: { offset: [.25, 0], rotation: .4, scale: [3, 2] } } } } },
    { name: 'Alpha mask', alphaMode: 'MASK', alphaCutoff: .5, doubleSided: true, pbrMetallicRoughness: { ...pbr([1, 1, 1, 1]), baseColorTexture: { index: 1 } } },
    { name: 'Alpha blend', alphaMode: 'BLEND', pbrMetallicRoughness: pbr([.3, .9, .5, .45], 0, .4) },
    { name: 'Normal and occlusion (second UV set)', normalTexture: { index: 2, scale: 1 }, occlusionTexture: { index: 4, texCoord: 1, strength: 1 }, pbrMetallicRoughness: pbr([.95, .8, .5, 1], 0, .5) },
    { name: 'Bump', pbrMetallicRoughness: pbr([.6, .6, .65, 1], .3, .5), extensions: { EXT_materials_bump: { bumpFactor: 1, bumpTexture: { index: 3 } } } },
    { name: 'WebP texture', pbrMetallicRoughness: { ...pbr([1, 1, 1, 1]), baseColorTexture: { index: 5 } } },
    { name: 'AVIF texture', pbrMetallicRoughness: { ...pbr([1, 1, 1, 1]), baseColorTexture: { index: 6 } } },
    { name: 'Points and lines', pbrMetallicRoughness: pbr([1, 1, 1, 1]) },
    { name: 'Flat (no normals)', pbrMetallicRoughness: pbr([.95, .75, .2, 1], 0, .6) },
    { name: 'Morph', pbrMetallicRoughness: pbr([.4, .8, 1, 1], 0, .4) },
    { name: 'Instanced', pbrMetallicRoughness: pbr([1, .85, .3, 1], .5, .3) },
  ];
  const material = (name: string) => { const index = materials.findIndex(item => item.name === name); if (index < 0) throw new Error(name); return index; };

  const meshes: any[] = [];
  const nodes: any[] = [];
  const place = (name: string, mesh: any, translation: number[], extra: any = {}) => {
    meshes.push({ name, ...mesh });
    nodes.push({ name, mesh: meshes.length - 1, translation, ...extra });
  };
  // A 5 x 4 grid, row-major from the top left.
  const at = (i: number) => [(i % 5 - 2) * .9, (1.5 - Math.floor(i / 5)) * .9, 0];
  const ballMaterials = ['Unlit', 'Clearcoat', 'Sheen', 'Iridescence', 'Transmission, volume, IOR', 'Specular, IOR', 'Anisotropy', 'Dispersion', 'Bump'];
  ballMaterials.forEach((name, i) => place(name, { primitives: [{ attributes: plain, indices: ballIndices, material: material(name) }] }, at(i)));
  place('Normal and occlusion (second UV set)', { primitives: [{ attributes: withUv1, indices: ballIndices, material: material('Normal and occlusion (second UV set)') }] }, at(9));
  for (const [index, name] of ['Texture transform', 'Alpha mask', 'Alpha blend', 'WebP texture', 'AVIF texture'].entries()) {
    place(name, { primitives: [{ attributes: cardAttributes, indices: cardIndices, material: material(name) }] }, at(10 + index));
  }
  // Points (mode 0) with vertex colors, a line loop (mode 2), and a line strip (mode 3).
  const points: number[] = [], pointColors: number[] = [];
  for (let i = 0; i < 64; i++) { const a = i * .7, r = .05 + i * .005; points.push(Math.cos(a) * r, Math.sin(a) * r, 0); pointColors.push(i / 63, .4, 1 - i / 63, 1); }
  const loop = Array.from({ length: 6 }, (_, i) => { const a = i * Math.PI / 3; return [Math.cos(a) * .3, Math.sin(a) * .3, 0]; }).flat();
  const strip = Array.from({ length: 12 }, (_, i) => [-.3 + i * .055, Math.sin(i) * .15, .05]).flat();
  place('Points and lines', { primitives: [
    { mode: 0, attributes: { POSITION: g.floats(points, 'VEC3', { minMax: true, target: 34962 }), COLOR_0: g.floats(pointColors, 'VEC4', { target: 34962 }) }, material: material('Points and lines') },
    { mode: 2, attributes: { POSITION: g.floats(loop, 'VEC3', { minMax: true, target: 34962 }) }, material: material('Points and lines') },
    { mode: 3, attributes: { POSITION: g.floats(strip, 'VEC3', { minMax: true, target: 34962 }) }, material: material('Points and lines') },
  ] }, at(15));
  // No NORMAL attribute: the loader computes flat normals. Positions are quantized
  // (KHR_mesh_quantization): SHORT values scaled down by the node.
  const corners = [[0, 400, 0], [-350, -200, 200], [350, -200, 200], [0, -200, -380]];
  const tetra = [[0, 1, 2], [0, 2, 3], [0, 3, 1], [1, 3, 2]].flatMap(face => {
    // Wind each face counter-clockwise seen from outside (its normal points away from the centroid).
    const [a, b, c] = face.map(i => corners[i]!) as [number[], number[], number[]];
    const u = [b[0]! - a[0]!, b[1]! - a[1]!, b[2]! - a[2]!], v = [c[0]! - a[0]!, c[1]! - a[1]!, c[2]! - a[2]!];
    const n = [u[1]! * v[2]! - u[2]! * v[1]!, u[2]! * v[0]! - u[0]! * v[2]!, u[0]! * v[1]! - u[1]! * v[0]!];
    const centroid = [0, 1, 2].map(k => (a[k]! + b[k]! + c[k]!) / 3 - corners.reduce((sum, p) => sum + p[k]!, 0) / 4);
    return (n[0]! * centroid[0]! + n[1]! * centroid[1]! + n[2]! * centroid[2]! >= 0 ? [a, b, c] : [a, c, b]).flat();
  });
  place('Flat (no normals)', { primitives: [{ attributes: { POSITION: g.typed(tetra, 5122, 'VEC3', { minMax: true, target: 34962 }) }, material: material('Flat (no normals)') }] }, at(16), { scale: [.001, .001, .001] });
  // A morph target bulging the sphere, animated by a weights clip.
  const bulge = ball.positions.map((value, i) => i % 3 === 1 ? 0 : value * .6);
  place('Morph', { primitives: [{ attributes: plain, indices: ballIndices, material: material('Morph'), targets: [{ POSITION: g.floats(bulge, 'VEC3', { minMax: true, target: 34962 }) }] }], weights: [0] }, at(17));
  // GPU instancing: four instances with their own translation, rotation, and scale.
  const quarter = (deg: number) => { const h = deg * Math.PI / 360; return [0, 0, Math.sin(h), Math.cos(h)]; };
  place('Instanced', { primitives: [{ attributes: cardAttributes, indices: cardIndices, material: material('Instanced') }] }, at(18), {
    scale: [.35, .35, .35],
    extensions: { EXT_mesh_gpu_instancing: { attributes: {
      TRANSLATION: g.floats([-.6, -.6, 0, .6, -.6, 0, -.6, .6, 0, .6, .6, 0], 'VEC3'),
      ROTATION: g.floats([0, 30, 60, 90].flatMap(quarter), 'VEC4'),
      SCALE: g.floats([1, 1, 1, .8, .8, .8, .6, .6, .6, 1.2, 1.2, 1.2], 'VEC3'),
    } } },
  });
  // Embedded punctual lights: a warm spot from the front and a cool point light.
  nodes.push({ name: 'Embedded spot', translation: [0, 0, 3], extensions: { KHR_lights_punctual: { light: 0 } } });
  nodes.push({ name: 'Embedded point', translation: [2, 1.5, 1], extensions: { KHR_lights_punctual: { light: 1 } } });
  const morphTimes = g.floats([0, 1, 2], 'SCALAR', { minMax: true });
  const morphWeights = g.floats([0, 1, 0], 'SCALAR');
  const bin = g.binary();
  const extensionsUsed = [...new Set([
    ...materials.flatMap(item => Object.keys((item as any).extensions ?? {})),
    'KHR_texture_transform', 'EXT_texture_webp', 'EXT_texture_avif', 'KHR_mesh_quantization', 'EXT_mesh_gpu_instancing', 'KHR_lights_punctual',
  ])].sort();
  const json = {
    asset: { version: '2.0', generator: 'Powermove engine corpus generator' },
    extensionsUsed,
    extensionsRequired: ['KHR_mesh_quantization'],
    extensions: { KHR_lights_punctual: { lights: [
      { name: 'Warm spot', type: 'spot', color: [1, .85, .6], intensity: 6, range: 10, spot: { innerConeAngle: .3, outerConeAngle: .6 } },
      { name: 'Cool point', type: 'point', color: [.5, .7, 1], intensity: 4 },
    ] } },
    scene: 0,
    scenes: [{ name: 'Materials', nodes: nodes.map((_, i) => i) }],
    nodes, meshes, materials, textures, images,
    samplers: [{ magFilter: 9728, minFilter: 9728, wrapS: 10497, wrapT: 10497 }, { magFilter: 9729, minFilter: 9987, wrapS: 10497, wrapT: 10497 }],
    animations: [{ name: 'Morph', samplers: [{ input: morphTimes, output: morphWeights, interpolation: 'LINEAR' }], channels: [{ sampler: 0, target: { node: nodes.findIndex(node => node.name === 'Morph'), path: 'weights' } }] }],
    accessors: g.accessors,
    bufferViews: g.bufferViews,
    buffers: [{ byteLength: bin.length }],
  };
  return packGlb(json, bin);
}

/** A Wavefront OBJ (a faceted gem with normals) for the legacy OBJ mesh layer. */
export function corpusObj(): string {
  const gem = octahedron(.9);
  const lines = ['# Powermove engine corpus OBJ', 'o Gem'];
  for (let i = 0; i < gem.positions.length; i += 3) lines.push(`v ${gem.positions.slice(i, i + 3).map(v => +v.toFixed(6)).join(' ')}`);
  for (let i = 0; i < gem.normals.length; i += 3) lines.push(`vn ${gem.normals.slice(i, i + 3).map(v => +v.toFixed(6)).join(' ')}`);
  for (let f = 0; f < gem.positions.length / 9; f++) {
    const a = f * 3 + 1;
    lines.push(`f ${a}//${a} ${a + 1}//${a + 1} ${a + 2}//${a + 2}`);
  }
  return lines.join('\n') + '\n';
}

/** An SVG with gradients, strokes, transforms, and a clip path (no text: usvg's text path is engine-side). */
export function corpusSvg(): string {
  return `<svg xmlns="http://www.w3.org/2000/svg" width="320" height="180" viewBox="0 0 320 180">
  <defs>
    <linearGradient id="sky" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0" stop-color="#1B2A4A"/><stop offset="0.6" stop-color="#4C8DFF"/><stop offset="1" stop-color="#FFD66B"/>
    </linearGradient>
    <radialGradient id="sun" cx="0.5" cy="0.5" r="0.5"><stop offset="0" stop-color="#FFF4D0"/><stop offset="1" stop-color="#FF6B1A" stop-opacity="0"/></radialGradient>
    <clipPath id="frame"><rect x="8" y="8" width="304" height="164" rx="18"/></clipPath>
  </defs>
  <g clip-path="url(#frame)">
    <rect width="320" height="180" fill="url(#sky)"/>
    <circle cx="230" cy="70" r="46" fill="url(#sun)"/>
    <path d="M0 150 C 60 110, 110 170, 170 130 S 270 100, 320 140 L 320 180 L 0 180 Z" fill="#0C0D12" opacity="0.85"/>
    <g transform="translate(70 60) rotate(-12)">
      <rect x="-30" y="-18" width="60" height="36" rx="6" fill="none" stroke="#E8E2CF" stroke-width="4" stroke-dasharray="10 6"/>
    </g>
  </g>
</svg>
`;
}
