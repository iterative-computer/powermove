export interface ObjMesh {
  /** Centered and normalized to a unit bounding sphere for predictable cameras. */
  positions: Float32Array;
  normals: Float32Array;
  texcoords: Float32Array;
  vertexCount: number;
  triangleCount: number;
  sourceVertexCount: number;
  bounds: {
    min: [number, number, number];
    max: [number, number, number];
    center: [number, number, number];
    radius: number;
  };
}

// CAD assemblies run to millions of triangles; geometry is held in typed
// arrays (about 36 bytes a triangle while parsing).
const MAX_SOURCE_VERTICES = 10_000_000;
const MAX_TRIANGLES = 10_000_000;

function finite(value: string, line: number): number {
  const number = Number(value);
  if (!Number.isFinite(number)) throw new Error(`OBJ line ${line}: expected a finite number`);
  return number;
}

/** Resolves a 1-based (or negative, relative) OBJ index to 0-based; -1 when absent. */
function resolveIndex(raw: string | undefined, length: number, line: number, label: string): number {
  if (raw == null || raw === '') return -1;
  const value = Number(raw);
  if (!Number.isInteger(value) || value === 0) throw new Error(`OBJ line ${line}: invalid ${label} index`);
  const index = value > 0 ? value - 1 : length + value;
  if (index < 0 || index >= length) throw new Error(`OBJ line ${line}: ${label} index is out of range`);
  return index;
}

type V3 = [number, number, number];

function faceNormal(a: V3, b: V3, c: V3): V3 {
  const abx = b[0] - a[0], aby = b[1] - a[1], abz = b[2] - a[2];
  const acx = c[0] - a[0], acy = c[1] - a[1], acz = c[2] - a[2];
  const x = aby * acz - abz * acy;
  const y = abz * acx - abx * acz;
  const z = abx * acy - aby * acx;
  const length = Math.hypot(x, y, z);
  return length > 1e-12 ? [x / length, y / length, z / length] : [0, 1, 0];
}

/** A growable typed array. */
class Grow<A extends Float64Array | Int32Array> {
  data: A;
  size = 0;
  constructor(make: (n: number) => A) { this.data = make(1024); }
  push3(a: number, b: number, c: number): void {
    if (this.size + 3 > this.data.length) {
      const next = new (this.data.constructor as any)(this.data.length * 2) as A;
      next.set(this.data);
      this.data = next;
    }
    this.data[this.size++] = a; this.data[this.size++] = b; this.data[this.size++] = c;
  }
}

/**
 * Parse the interoperable core of Wavefront OBJ: v/vt/vn plus polygonal faces,
 * including negative indices. Faces are triangulated as a fan. Material names
 * are intentionally left in the source asset for a later MTL capability; this
 * renderer uses one editable Powermove material per layer.
 */
export function parseObj(source: string): ObjMesh {
  if (typeof source !== 'string' || !source.trim()) throw new Error('OBJ file is empty');
  const vertices = new Grow(n => new Float64Array(n));
  const normals = new Grow(n => new Float64Array(n));
  const texcoords = new Grow(n => new Float64Array(n));
  // Per triangle corner: vertex, texture coordinate and normal indices (-1: none).
  const corners = new Grow(n => new Int32Array(n));
  const lines = source.replace(/\r\n?/g, '\n').split('\n');
  const face: number[] = [];

  for (let index = 0; index < lines.length; index++) {
    const lineNumber = index + 1;
    const line = lines[index]!.split('#', 1)[0]!.trim();
    if (!line) continue;
    const fields = line.split(/\s+/);
    const kind = fields[0];
    if (kind === 'v') {
      if (fields.length < 4) throw new Error(`OBJ line ${lineNumber}: vertex needs x, y, and z`);
      vertices.push3(finite(fields[1]!, lineNumber), finite(fields[2]!, lineNumber), finite(fields[3]!, lineNumber));
      if (vertices.size / 3 > MAX_SOURCE_VERTICES) throw new Error('OBJ has too many vertices');
    } else if (kind === 'vn') {
      if (fields.length < 4) throw new Error(`OBJ line ${lineNumber}: normal needs x, y, and z`);
      const x = finite(fields[1]!, lineNumber), y = finite(fields[2]!, lineNumber), z = finite(fields[3]!, lineNumber);
      const length = Math.hypot(x, y, z);
      if (length > 1e-12) normals.push3(x / length, y / length, z / length); else normals.push3(0, 1, 0);
    } else if (kind === 'vt') {
      if (fields.length < 3) throw new Error(`OBJ line ${lineNumber}: texture coordinate needs u and v`);
      texcoords.push3(finite(fields[1]!, lineNumber), finite(fields[2]!, lineNumber), 0);
    } else if (kind === 'f') {
      if (fields.length < 4) throw new Error(`OBJ line ${lineNumber}: face needs at least three vertices`);
      face.length = 0;
      for (let k = 1; k < fields.length; k++) {
        const [v, vt, vn] = fields[k]!.split('/');
        face.push(
          resolveIndex(v, vertices.size / 3, lineNumber, 'vertex'),
          resolveIndex(vt, texcoords.size / 3, lineNumber, 'texture coordinate'),
          resolveIndex(vn, normals.size / 3, lineNumber, 'normal'),
        );
        if (face[face.length - 3] === -1) throw new Error(`OBJ line ${lineNumber}: invalid vertex index`);
      }
      // A fan from the first corner.
      for (let corner = 1; corner < face.length / 3 - 1; corner++) {
        for (const c of [0, corner, corner + 1]) corners.push3(face[c * 3]!, face[c * 3 + 1]!, face[c * 3 + 2]!);
        if (corners.size / 9 > MAX_TRIANGLES) throw new Error('OBJ has too many triangles');
      }
    }
  }

  const vertexCount = vertices.size / 3, triangleCount = corners.size / 9;
  if (!vertexCount) throw new Error('OBJ does not contain any vertices');
  if (!triangleCount) throw new Error('OBJ does not contain any faces');
  const vs = vertices.data, ns = normals.data, ts = texcoords.data, cs = corners.data;

  const min: V3 = [Infinity, Infinity, Infinity], max: V3 = [-Infinity, -Infinity, -Infinity];
  for (let i = 0; i < vertexCount; i++) for (let a = 0; a < 3; a++) {
    const x = vs[i * 3 + a]!;
    if (x < min[a]!) min[a] = x;
    if (x > max[a]!) max[a] = x;
  }
  const center: V3 = [(min[0] + max[0]) / 2, (min[1] + max[1]) / 2, (min[2] + max[2]) / 2];
  let radius = 0;
  for (let i = 0; i < vertexCount; i++) radius = Math.max(radius, Math.hypot(vs[i * 3]! - center[0], vs[i * 3 + 1]! - center[1], vs[i * 3 + 2]! - center[2]));
  if (!(radius > 1e-12)) throw new Error('OBJ geometry has no measurable size');

  const outPositions = new Float32Array(triangleCount * 9);
  const outNormals = new Float32Array(triangleCount * 9);
  const outTexcoords = new Float32Array(triangleCount * 6);
  const vertex = (i: number): V3 => [vs[i * 3]!, vs[i * 3 + 1]!, vs[i * 3 + 2]!];
  for (let t = 0; t < triangleCount; t++) {
    const fallbackNormal = faceNormal(vertex(cs[t * 9]!), vertex(cs[t * 9 + 3]!), vertex(cs[t * 9 + 6]!));
    for (let c = 0; c < 3; c++) {
      const o = t * 9 + c * 3, v = cs[o]!, vt = cs[o + 1]!, vn = cs[o + 2]!;
      for (let a = 0; a < 3; a++) {
        outPositions[t * 9 + c * 3 + a] = (vs[v * 3 + a]! - center[a]!) / radius;
        outNormals[t * 9 + c * 3 + a] = vn < 0 ? fallbackNormal[a]! : ns[vn * 3 + a]!;
      }
      outTexcoords[t * 6 + c * 2] = vt < 0 ? 0 : ts[vt * 3]!;
      outTexcoords[t * 6 + c * 2 + 1] = vt < 0 ? 0 : ts[vt * 3 + 1]!;
    }
  }

  return {
    positions: outPositions,
    normals: outNormals,
    texcoords: outTexcoords,
    vertexCount: triangleCount * 3,
    triangleCount,
    sourceVertexCount: vertexCount,
    bounds: { min, max, center, radius }
  };
}
