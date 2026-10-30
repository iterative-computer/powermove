import * as THREE from 'three';

/*
 * Gaussian splats: captured scenes and products as millions of soft, oriented
 * ellipsoids. Reads 3DGS PLY (spherical-harmonic DC color, log scales,
 * quaternions) and the 32-byte `.splat` layout; draws them rasterized, each a
 * screen-space ellipse from its projected 3D covariance (EWA), sorted back to
 * front. Studio leaves splat layers to this renderer and composites them by
 * layer order.
 */

export type SplatData = {
  count: number;
  centers: Float32Array;   // xyz
  covariances: Float32Array; // xx xy xz yy yz zz
  colors: Uint8Array;      // sRGB rgba (alpha = opacity)
};

export const SPLAT_FILE = /\.(ply|splat)$/i;
const SH_C0 = 0.28209479177387814;
const MAX_SPLATS = 16_000_000;

function covariance(out: Float32Array, o: number, sx: number, sy: number, sz: number, qw: number, qx: number, qy: number, qz: number): void {
  const l = Math.hypot(qw, qx, qy, qz) || 1;
  const w = qw / l, x = qx / l, y = qy / l, z = qz / l;
  // R · S, then Σ = (R S)(R S)ᵀ.
  const r = [
    1 - 2 * (y * y + z * z), 2 * (x * y - w * z), 2 * (x * z + w * y),
    2 * (x * y + w * z), 1 - 2 * (x * x + z * z), 2 * (y * z - w * x),
    2 * (x * z - w * y), 2 * (y * z + w * x), 1 - 2 * (x * x + y * y),
  ];
  const m = [r[0]! * sx, r[1]! * sy, r[2]! * sz, r[3]! * sx, r[4]! * sy, r[5]! * sz, r[6]! * sx, r[7]! * sy, r[8]! * sz];
  const dot = (a: number, b: number) => m[a * 3]! * m[b * 3]! + m[a * 3 + 1]! * m[b * 3 + 1]! + m[a * 3 + 2]! * m[b * 3 + 2]!;
  out[o] = dot(0, 0); out[o + 1] = dot(0, 1); out[o + 2] = dot(0, 2);
  out[o + 3] = dot(1, 1); out[o + 4] = dot(1, 2); out[o + 5] = dot(2, 2);
}

function allocate(count: number): SplatData {
  if (!count) throw new Error('The splat file has no splats');
  if (count > MAX_SPLATS) throw new Error('The splat file has more than 16 million splats');
  return { count, centers: new Float32Array(count * 3), covariances: new Float32Array(count * 6), colors: new Uint8Array(count * 4) };
}

/** antimatter15's `.splat`: position, scale (f32 ×3 each), rgba (u8 ×4), rotation (u8 ×4, w x y z). */
export function parseSplat(buffer: ArrayBuffer): SplatData {
  if (buffer.byteLength % 32) throw new Error('Not a .splat file (32 bytes per splat)');
  const n = buffer.byteLength / 32, d = allocate(n), f = new Float32Array(buffer), u = new Uint8Array(buffer);
  for (let i = 0; i < n; i++) {
    d.centers.set(f.subarray(i * 8, i * 8 + 3), i * 3);
    d.colors.set(u.subarray(i * 32 + 24, i * 32 + 28), i * 4);
    const q = (k: number) => (u[i * 32 + 28 + k]! - 128) / 128;
    covariance(d.covariances, i * 6, f[i * 8 + 3]!, f[i * 8 + 4]!, f[i * 8 + 5]!, q(0), q(1), q(2), q(3));
  }
  return d;
}

const PLY_TYPES: Record<string, [number, (v: DataView, o: number) => number]> = {
  float: [4, (v, o) => v.getFloat32(o, true)], float32: [4, (v, o) => v.getFloat32(o, true)],
  double: [8, (v, o) => v.getFloat64(o, true)], float64: [8, (v, o) => v.getFloat64(o, true)],
  uchar: [1, (v, o) => v.getUint8(o)], uint8: [1, (v, o) => v.getUint8(o)], char: [1, (v, o) => v.getInt8(o)], int8: [1, (v, o) => v.getInt8(o)],
  ushort: [2, (v, o) => v.getUint16(o, true)], uint16: [2, (v, o) => v.getUint16(o, true)], short: [2, (v, o) => v.getInt16(o, true)], int16: [2, (v, o) => v.getInt16(o, true)],
  uint: [4, (v, o) => v.getUint32(o, true)], uint32: [4, (v, o) => v.getUint32(o, true)], int: [4, (v, o) => v.getInt32(o, true)], int32: [4, (v, o) => v.getInt32(o, true)],
};

/**
 * Binary little-endian PLY: 3D Gaussian Splatting's layout (x y z, f_dc_0..2,
 * opacity, scale_0..2, rot_0..3), or a plain colored point cloud (red green
 * blue), whose points become small round splats.
 */
export function parsePly(buffer: ArrayBuffer): SplatData {
  const bytes = new Uint8Array(buffer);
  const head = new TextDecoder().decode(bytes.subarray(0, Math.min(bytes.length, 64 * 1024)));
  const end = head.indexOf('end_header');
  if (!head.startsWith('ply') || end < 0) throw new Error('Not a PLY file');
  const lines = head.slice(0, end).split(/\r?\n/);
  if (!lines.some(l => l.startsWith('format binary_little_endian'))) throw new Error('Only binary little-endian PLY splats are supported');
  let count = 0, inVertex = false, stride = 0;
  const props = new Map<string, [number, (v: DataView, o: number) => number]>();
  for (const line of lines) {
    const p = line.trim().split(/\s+/);
    if (p[0] === 'element') { inVertex = p[1] === 'vertex'; if (inVertex) count = Number(p[2]); continue; }
    if (p[0] === 'property' && inVertex) {
      if (p[1] === 'list') throw new Error('PLY vertex lists are not supported');
      const t = PLY_TYPES[p[1]!];
      if (!t) throw new Error(`Unsupported PLY type ${p[1]}`);
      props.set(p[2]!, [stride, t[1]]);
      stride += t[0];
    }
  }
  const start = new TextEncoder().encode(head.slice(0, end + 'end_header'.length)).length + 1;
  if (start + count * stride > buffer.byteLength) throw new Error('The PLY file is truncated');
  const d = allocate(count), view = new DataView(buffer, start);
  const get = (name: string) => props.get(name);
  const [px, py, pz] = ['x', 'y', 'z'].map(get);
  if (!px || !py || !pz) throw new Error('The PLY file has no positions');
  const dc = ['f_dc_0', 'f_dc_1', 'f_dc_2'].map(get), rgb = ['red', 'green', 'blue'].map(get);
  const opacity = get('opacity'), scales = ['scale_0', 'scale_1', 'scale_2'].map(get), rot = ['rot_0', 'rot_1', 'rot_2', 'rot_3'].map(get);
  const gaussian = dc.every(Boolean) && scales.every(Boolean) && rot.every(Boolean);
  const read = (p: [number, (v: DataView, o: number) => number], i: number) => p[1](view, i * stride + p[0]);
  for (let i = 0; i < count; i++) {
    d.centers[i * 3] = read(px, i); d.centers[i * 3 + 1] = read(py, i); d.centers[i * 3 + 2] = read(pz, i);
  }
  // Point clouds: round splats about half the typical point spacing.
  let pointSize = 0;
  if (!gaussian) {
    const box = new THREE.Box3().setFromArray(d.centers);
    pointSize = 0.5 * box.getSize(new THREE.Vector3()).length() / Math.cbrt(count) / 2;
  }
  const clamp8 = (v: number) => Math.max(0, Math.min(255, Math.round(v * 255)));
  for (let i = 0; i < count; i++) {
    const c = i * 4;
    if (gaussian) {
      for (let k = 0; k < 3; k++) d.colors[c + k] = clamp8(0.5 + SH_C0 * read(dc[k]!, i));
      d.colors[c + 3] = clamp8(opacity ? 1 / (1 + Math.exp(-read(opacity, i))) : 1);
      covariance(d.covariances, i * 6, Math.exp(read(scales[0]!, i)), Math.exp(read(scales[1]!, i)), Math.exp(read(scales[2]!, i)),
        read(rot[0]!, i), read(rot[1]!, i), read(rot[2]!, i), read(rot[3]!, i));
    } else {
      for (let k = 0; k < 3; k++) d.colors[c + k] = rgb[k] ? read(rgb[k]!, i) : 200;
      d.colors[c + 3] = 255;
      covariance(d.covariances, i * 6, pointSize, pointSize, pointSize, 1, 0, 0, 0);
    }
  }
  return d;
}

const vertexShader = /* glsl */ `
precision highp float;
precision highp int;
uniform highp sampler2D tCenter;
uniform highp sampler2D tCovA;
uniform highp sampler2D tCovB;
uniform highp sampler2D tColor;
uniform vec2 viewport;
uniform int width;
in vec3 position;
in float splatIndex;
uniform mat4 modelViewMatrix;
uniform mat4 projectionMatrix;
out vec4 vColor;
out vec2 vPos;
void main() {
  int i = int(splatIndex);
  ivec2 uv = ivec2(i % width, i / width);
  vec4 cam = modelViewMatrix * vec4(texelFetch(tCenter, uv, 0).xyz, 1.0);
  vec4 clip = projectionMatrix * cam;
  float d = -cam.z;
  float b = 1.2 * clip.w;
  if (d <= 0.0 || clip.x < -b || clip.x > b || clip.y < -b || clip.y > b) { gl_Position = vec4(0.0, 0.0, 2.0, 1.0); return; }
  vec4 a = texelFetch(tCovA, uv, 0);
  vec2 c = texelFetch(tCovB, uv, 0).xy;
  mat3 sigma = mat3(a.x, a.y, a.z, a.y, a.w, c.x, a.z, c.x, c.y);
  float fx = projectionMatrix[0][0] * viewport.x * 0.5, fy = projectionMatrix[1][1] * viewport.y * 0.5;
  // Jacobian of the perspective projection at the splat (rows, then transposed into columns).
  mat3 J = transpose(mat3(fx / d, 0.0, fx * cam.x / (d * d), 0.0, fy / d, fy * cam.y / (d * d), 0.0, 0.0, 0.0));
  mat3 T = J * mat3(modelViewMatrix);
  mat3 cov = T * sigma * transpose(T);
  float xx = cov[0][0] + 0.3, xy = cov[0][1], yy = cov[1][1] + 0.3;
  float mid = 0.5 * (xx + yy), radius = length(vec2(0.5 * (xx - yy), xy));
  float l1 = mid + radius, l2 = max(mid - radius, 0.1);
  vec2 axis = abs(xy) < 1e-8 ? (xx >= yy ? vec2(1.0, 0.0) : vec2(0.0, 1.0)) : normalize(vec2(xy, l1 - xx));
  vec2 major = min(sqrt(2.0 * l1), 1024.0) * axis;
  vec2 minor = min(sqrt(2.0 * l2), 1024.0) * vec2(axis.y, -axis.x);
  vPos = position.xy;
  vColor = texelFetch(tColor, uv, 0);
  vec2 offset = (position.x * major + position.y * minor) * 2.0 / viewport;
  gl_Position = vec4(clip.xy / clip.w + offset, clip.z / clip.w, 1.0);
}`;

const fragmentShader = /* glsl */ `
precision highp float;
in vec4 vColor;
in vec2 vPos;
out vec4 fragColor;
void main() {
  float r = -dot(vPos, vPos);
  if (r < -4.0) discard;
  float alpha = exp(r) * vColor.a;
  if (alpha < 1.0 / 255.0) discard;
  fragColor = vec4(vColor.rgb * alpha, alpha);
}`;

const TEX_WIDTH = 2048;

function texture(data: Float32Array | Uint8Array, count: number, floats: boolean): THREE.DataTexture {
  const h = Math.max(1, Math.ceil(count / TEX_WIDTH));
  const t = new THREE.DataTexture(data, TEX_WIDTH, h, THREE.RGBAFormat, floats ? THREE.FloatType : THREE.UnsignedByteType);
  t.needsUpdate = true;
  return t;
}

/** A splat cloud as one three.js object; sorts itself back to front for each new view. */
export class SplatMesh extends THREE.Mesh<THREE.InstancedBufferGeometry, THREE.RawShaderMaterial> {
  data: SplatData | null = null;
  private lastView = new THREE.Matrix4();
  private depths: Float32Array | null = null;

  constructor(data?: SplatData) {
    super(new THREE.InstancedBufferGeometry(), new THREE.RawShaderMaterial());
    this.userData.splat = true;
    this.frustumCulled = false;
    this.castShadow = false;
    if (!data) return;
    this.data = data;
    const n = data.count, rows = Math.ceil(n / TEX_WIDTH) * TEX_WIDTH;
    const center = new Float32Array(rows * 4), covA = new Float32Array(rows * 4), covB = new Float32Array(rows * 4), color = new Uint8Array(rows * 4);
    for (let i = 0; i < n; i++) {
      center[i * 4] = data.centers[i * 3]!; center[i * 4 + 1] = data.centers[i * 3 + 1]!; center[i * 4 + 2] = data.centers[i * 3 + 2]!;
      covA.set(data.covariances.subarray(i * 6, i * 6 + 4), i * 4);
      covB[i * 4] = data.covariances[i * 6 + 4]!; covB[i * 4 + 1] = data.covariances[i * 6 + 5]!;
    }
    color.set(data.colors);
    const g = this.geometry;
    // One quad per splat spanning ±2 of its standard axes.
    g.setAttribute('position', new THREE.BufferAttribute(new Float32Array([-2, -2, 0, 2, -2, 0, 2, 2, 0, -2, 2, 0]), 3));
    g.setIndex([0, 1, 2, 0, 2, 3]);
    g.setAttribute('splatIndex', new THREE.InstancedBufferAttribute(new Float32Array(n), 1));
    g.instanceCount = n;
    g.boundingSphere = new THREE.Sphere().setFromPoints([new THREE.Box3().setFromArray(data.centers).min, new THREE.Box3().setFromArray(data.centers).max]);
    this.material = new THREE.RawShaderMaterial({
      vertexShader, fragmentShader, glslVersion: THREE.GLSL3,
      uniforms: {
        tCenter: { value: texture(center, n, true) }, tCovA: { value: texture(covA, n, true) }, tCovB: { value: texture(covB, n, true) },
        tColor: { value: texture(color, n, false) }, viewport: { value: new THREE.Vector2(1, 1) }, width: { value: TEX_WIDTH },
      },
      // The ellipse's axes mirror the quad, so its winding flips: draw both sides.
      transparent: true, depthWrite: false, depthTest: true, side: THREE.DoubleSide,
      blending: THREE.CustomBlending, blendSrc: THREE.OneFactor, blendDst: THREE.OneMinusSrcAlphaFactor,
      blendSrcAlpha: THREE.OneFactor, blendDstAlpha: THREE.OneMinusSrcAlphaFactor, premultipliedAlpha: true,
    });
  }

  /** Clones share the splat data, textures and shader (copied objects own only their sort order). */
  override copy(source: this, recursive?: boolean): this {
    super.copy(source, recursive);
    this.data = source.data;
    this.geometry = source.geometry.clone() as THREE.InstancedBufferGeometry;
    this.material = source.material;
    return this;
  }

  override onBeforeRender(renderer: THREE.WebGLRenderer, _scene: THREE.Scene, camera: THREE.Camera): void {
    if (!this.data) return;
    const size = renderer.getDrawingBufferSize(new THREE.Vector2());
    (this.material.uniforms.viewport!.value as THREE.Vector2).copy(size);
    const view = new THREE.Matrix4().multiplyMatrices(camera.matrixWorldInverse, this.matrixWorld);
    if (view.equals(this.lastView)) return;
    this.lastView.copy(view);
    this.sort(view);
  }

  /** Back-to-front order for this view: a 16-bit counting sort on view depth. */
  private sort(view: THREE.Matrix4): void {
    const d = this.data!, n = d.count, e = view.elements;
    const depths = (this.depths ??= new Float32Array(n));
    let lo = Infinity, hi = -Infinity;
    for (let i = 0; i < n; i++) {
      // View-space z (negative in front of the camera).
      const z = e[2]! * d.centers[i * 3]! + e[6]! * d.centers[i * 3 + 1]! + e[10]! * d.centers[i * 3 + 2]! + e[14]!;
      depths[i] = z;
      if (z < lo) lo = z;
      if (z > hi) hi = z;
    }
    const buckets = 65536, scale = (buckets - 1) / Math.max(hi - lo, 1e-9);
    const counts = new Uint32Array(buckets), keys = new Uint16Array(n);
    for (let i = 0; i < n; i++) { const k = ((depths[i]! - lo) * scale) | 0; keys[i] = k; counts[k]!++; }
    // Most negative z (farthest) first.
    for (let k = 1; k < buckets; k++) counts[k]! += counts[k - 1]!;
    const attr = this.geometry.getAttribute('splatIndex') as THREE.InstancedBufferAttribute, order = attr.array as Float32Array;
    for (let i = n - 1; i >= 0; i--) order[--counts[keys[i]!]!] = i;
    attr.needsUpdate = true;
  }
}

/** Parses a splat file into an object normalized like other models (unit radius around its dense core). */
export async function loadSplats(blob: Blob, name: string): Promise<{ object3d: THREE.Group; vertices: number; triangles: number; splats: number }> {
  const buffer = await blob.arrayBuffer();
  const data = /\.splat$/i.test(name) ? parseSplat(buffer) : parsePly(buffer);
  // Captures carry stray far-away splats: frame the 2nd–98th percentile.
  const axis = (a: number) => {
    const v = new Float32Array(data.count);
    for (let i = 0; i < data.count; i++) v[i] = data.centers[i * 3 + a]!;
    v.sort();
    return [v[Math.floor(data.count * 0.02)]!, v[Math.min(data.count - 1, Math.floor(data.count * 0.98))]!] as const;
  };
  const [x, y, z] = [axis(0), axis(1), axis(2)];
  const center = new THREE.Vector3((x[0] + x[1]) / 2, (y[0] + y[1]) / 2, (z[0] + z[1]) / 2);
  const radius = Math.max(1e-6, Math.hypot(x[1] - x[0], y[1] - y[0], z[1] - z[0]) / 2);
  const mesh = new SplatMesh(data);
  mesh.name = name;
  // 3DGS captures (COLMAP) are y-down; Powermove is y-up.
  const normalization = new THREE.Group();
  normalization.rotation.x = Math.PI;
  normalization.scale.setScalar(1 / radius);
  mesh.position.copy(center).negate();
  normalization.add(mesh);
  const root = new THREE.Group();
  root.add(normalization);
  return { object3d: root, vertices: data.count, triangles: 0, splats: data.count };
}
