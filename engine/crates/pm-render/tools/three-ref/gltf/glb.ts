// Tiny glTF 2.0 builder: accessors, buffer views, images, and a GLB container.
// The corpus authors its models in code so every feature is explicit and reproducible.

type Json = Record<string, any>;
const COMPONENT = { i8: 5120, u8: 5121, i16: 5122, u16: 5123, u32: 5125, f32: 5126 } as const;
type Component = keyof typeof COMPONENT;
const TYPE_SIZE: Record<string, number> = { SCALAR: 1, VEC2: 2, VEC3: 3, VEC4: 4, MAT4: 16 };

export class Gltf {
  json: Json = { asset: { version: '2.0', generator: 'powermove-gltf-corpus' }, scenes: [{ nodes: [] }], scene: 0,
    nodes: [], meshes: [], accessors: [], bufferViews: [], buffers: [], materials: [] };
  private chunks: Uint8Array[] = [];
  private length = 0;

  private align(n: number) { while (this.length % n) { this.chunks.push(new Uint8Array(1)); this.length++; } }

  bufferView(bytes: Uint8Array, target?: number, stride?: number): number {
    this.align(4);
    const view: Json = { buffer: 0, byteOffset: this.length, byteLength: bytes.length };
    if (target) view.target = target;
    if (stride) view.byteStride = stride;
    this.chunks.push(bytes); this.length += bytes.length;
    this.json.bufferViews.push(view);
    return this.json.bufferViews.length - 1;
  }

  /** Adds an accessor; `normalized` integer data is used for KHR_mesh_quantization. */
  accessor(values: ArrayLike<number>, type: string, component: Component = 'f32', options: { normalized?: boolean; target?: number; minmax?: boolean } = {}): number {
    const Ctor = { i8: Int8Array, u8: Uint8Array, i16: Int16Array, u16: Uint16Array, u32: Uint32Array, f32: Float32Array }[component];
    const size = TYPE_SIZE[type]!, count = values.length / size;
    const elementBytes = Ctor.BYTES_PER_ELEMENT * size;
    // Vertex attributes need 4-byte aligned elements; pad each element when required.
    const stride = options.target === 34962 && elementBytes % 4 ? elementBytes + (4 - (elementBytes % 4)) : undefined;
    let bytes: Uint8Array;
    if (stride) {
      bytes = new Uint8Array(stride * count);
      for (let i = 0; i < count; i++) {
        const element = new Ctor(size);
        for (let k = 0; k < size; k++) element[k] = values[i * size + k]!;
        bytes.set(new Uint8Array(element.buffer), i * stride);
      }
    } else bytes = new Uint8Array(new Ctor(Array.from(values)).buffer);
    const accessor: Json = { bufferView: this.bufferView(bytes, options.target, stride), componentType: COMPONENT[component], count, type };
    if (options.normalized) accessor.normalized = true;
    if (options.minmax !== false && type !== 'MAT4') {
      const min = new Array(size).fill(Infinity), max = new Array(size).fill(-Infinity);
      for (let i = 0; i < count; i++) for (let k = 0; k < size; k++) {
        const v = values[i * size + k]!;
        min[k] = Math.min(min[k], v); max[k] = Math.max(max[k], v);
      }
      if (options.target === 34962 || type === 'SCALAR') { accessor.min = min; accessor.max = max; }
    }
    this.json.accessors.push(accessor);
    return this.json.accessors.length - 1;
  }

  image(bytes: Uint8Array, mimeType: string): number {
    this.json.images ??= [];
    this.json.images.push({ bufferView: this.bufferView(bytes), mimeType });
    return this.json.images.length - 1;
  }

  texture(image: number, sampler: Json = { magFilter: 9729, minFilter: 9987, wrapS: 10497, wrapT: 10497 }, extensions?: Json): number {
    this.json.samplers ??= [];
    this.json.textures ??= [];
    this.json.samplers.push(sampler);
    const texture: Json = { sampler: this.json.samplers.length - 1, source: image };
    if (extensions) { texture.extensions = extensions; delete texture.source; }
    this.json.textures.push(texture);
    return this.json.textures.length - 1;
  }

  material(material: Json): number { this.json.materials.push(material); return this.json.materials.length - 1; }
  mesh(mesh: Json): number { this.json.meshes.push(mesh); return this.json.meshes.length - 1; }
  node(node: Json, root = true): number {
    this.json.nodes.push(node);
    const index = this.json.nodes.length - 1;
    if (root) this.json.scenes[0].nodes.push(index);
    return index;
  }
  useExtension(name: string, required = false) {
    this.json.extensionsUsed ??= [];
    if (!this.json.extensionsUsed.includes(name)) this.json.extensionsUsed.push(name);
    if (required) { this.json.extensionsRequired ??= []; if (!this.json.extensionsRequired.includes(name)) this.json.extensionsRequired.push(name); }
  }

  glb(): Uint8Array {
    this.align(4);
    const bin = new Uint8Array(this.length);
    let offset = 0;
    for (const c of this.chunks) { bin.set(c, offset); offset += c.length; }
    this.json.buffers = [{ byteLength: bin.length }];
    if (!this.json.materials.length) delete this.json.materials;
    let text = new TextEncoder().encode(JSON.stringify(this.json));
    const pad = (4 - (text.length % 4)) % 4;
    const jsonChunk = new Uint8Array(text.length + pad).fill(0x20);
    jsonChunk.set(text);
    const total = 12 + 8 + jsonChunk.length + 8 + bin.length;
    const out = new Uint8Array(total), view = new DataView(out.buffer);
    view.setUint32(0, 0x46546c67, true); view.setUint32(4, 2, true); view.setUint32(8, total, true);
    view.setUint32(12, jsonChunk.length, true); view.setUint32(16, 0x4e4f534a, true);
    out.set(jsonChunk, 20);
    view.setUint32(20 + jsonChunk.length, bin.length, true); view.setUint32(24 + jsonChunk.length, 0x004e4942, true);
    out.set(bin, 28 + jsonChunk.length);
    return out;
  }
}

/** Indexed UV sphere matching the conventional layout (poles duplicated per segment). */
export function sphere(radius: number, widthSegments: number, heightSegments: number) {
  const positions: number[] = [], normals: number[] = [], uvs: number[] = [], indices: number[] = [];
  for (let iy = 0; iy <= heightSegments; iy++) {
    const v = iy / heightSegments;
    for (let ix = 0; ix <= widthSegments; ix++) {
      const u = ix / widthSegments;
      const x = -Math.cos(u * Math.PI * 2) * Math.sin(v * Math.PI);
      const y = Math.cos(v * Math.PI);
      const z = Math.sin(u * Math.PI * 2) * Math.sin(v * Math.PI);
      positions.push(x * radius, y * radius, z * radius);
      normals.push(x, y, z);
      uvs.push(u, v); // glTF UV origin is the top-left of the image
    }
  }
  const row = widthSegments + 1;
  for (let iy = 0; iy < heightSegments; iy++) for (let ix = 0; ix < widthSegments; ix++) {
    const a = iy * row + ix + 1, b = iy * row + ix, c = (iy + 1) * row + ix, d = (iy + 1) * row + ix + 1;
    if (iy !== 0) indices.push(a, b, d);
    if (iy !== heightSegments - 1) indices.push(b, c, d);
  }
  return { positions, normals, uvs, indices };
}

/** A capped cylinder along +Y from y0 to y1, rings at every `rings` step (for skinning). */
export function tube(radius: number, y0: number, y1: number, radial: number, rings: number) {
  const positions: number[] = [], normals: number[] = [], uvs: number[] = [], indices: number[] = [];
  for (let r = 0; r <= rings; r++) {
    const t = r / rings, y = y0 + (y1 - y0) * t;
    for (let s = 0; s <= radial; s++) {
      const a = (s / radial) * Math.PI * 2;
      positions.push(Math.sin(a) * radius, y, Math.cos(a) * radius);
      normals.push(Math.sin(a), 0, Math.cos(a));
      uvs.push(s / radial, 1 - t);
    }
  }
  const row = radial + 1;
  for (let r = 0; r < rings; r++) for (let s = 0; s < radial; s++) {
    const a = r * row + s, b = a + 1, c = a + row, d = c + 1;
    indices.push(a, b, d, a, d, c);
  }
  return { positions, normals, uvs, indices };
}
