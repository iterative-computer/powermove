/**
 * A small, deterministic binary glTF 2.0 file exercising the loader paths the
 * 3D corpus needs without Blender: an embedded PNG base-color texture with UVs,
 * vertex colors, emissive strength, a two-joint skinned mesh, and two
 * animation clips (three.js plays every clip at weight 1, looping).
 */
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
