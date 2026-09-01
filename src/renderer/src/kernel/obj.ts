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

const MAX_SOURCE_VERTICES = 1_000_000;
const MAX_TRIANGLES = 500_000;

type V3 = [number, number, number];
type V2 = [number, number];

function finite(value: string, line: number): number {
  const number = Number(value);
  if (!Number.isFinite(number)) throw new Error(`OBJ line ${line}: expected a finite number`);
  return number;
}

function resolveIndex(raw: string | undefined, length: number, line: number, label: string): number | null {
  if (raw == null || raw === '') return null;
  const value = Number(raw);
  if (!Number.isInteger(value) || value === 0) throw new Error(`OBJ line ${line}: invalid ${label} index`);
  const index = value > 0 ? value - 1 : length + value;
  if (index < 0 || index >= length) throw new Error(`OBJ line ${line}: ${label} index is out of range`);
  return index;
}

function faceNormal(a: V3, b: V3, c: V3): V3 {
  const abx = b[0] - a[0], aby = b[1] - a[1], abz = b[2] - a[2];
  const acx = c[0] - a[0], acy = c[1] - a[1], acz = c[2] - a[2];
  const x = aby * acz - abz * acy;
  const y = abz * acx - abx * acz;
  const z = abx * acy - aby * acx;
  const length = Math.hypot(x, y, z);
  return length > 1e-12 ? [x / length, y / length, z / length] : [0, 1, 0];
}

/**
 * Parse the interoperable core of Wavefront OBJ: v/vt/vn plus polygonal faces,
 * including negative indices. Faces are triangulated as a fan. Material names
 * are intentionally left in the source asset for a later MTL capability; this
 * renderer uses one editable Powermove material per layer.
 */
export function parseObj(source: string): ObjMesh {
  if (typeof source !== 'string' || !source.trim()) throw new Error('OBJ file is empty');
  const vertices: V3[] = [];
  const normals: V3[] = [];
  const texcoords: V2[] = [];
  const triangles: Array<Array<{ v: number; vt: number | null; vn: number | null }>> = [];
  const lines = source.replace(/\r\n?/g, '\n').split('\n');

  for (let index = 0; index < lines.length; index++) {
    const lineNumber = index + 1;
    const line = lines[index]!.split('#', 1)[0]!.trim();
    if (!line || line.startsWith('#')) continue;
    const fields = line.split(/\s+/);
    const kind = fields.shift();
    if (kind === 'v') {
      if (fields.length < 3) throw new Error(`OBJ line ${lineNumber}: vertex needs x, y, and z`);
      vertices.push([finite(fields[0]!, lineNumber), finite(fields[1]!, lineNumber), finite(fields[2]!, lineNumber)]);
      if (vertices.length > MAX_SOURCE_VERTICES) throw new Error('OBJ has too many vertices');
    } else if (kind === 'vn') {
      if (fields.length < 3) throw new Error(`OBJ line ${lineNumber}: normal needs x, y, and z`);
      const raw: V3 = [finite(fields[0]!, lineNumber), finite(fields[1]!, lineNumber), finite(fields[2]!, lineNumber)];
      const length = Math.hypot(...raw);
      normals.push(length > 1e-12 ? [raw[0] / length, raw[1] / length, raw[2] / length] : [0, 1, 0]);
    } else if (kind === 'vt') {
      if (fields.length < 2) throw new Error(`OBJ line ${lineNumber}: texture coordinate needs u and v`);
      texcoords.push([finite(fields[0]!, lineNumber), finite(fields[1]!, lineNumber)]);
    } else if (kind === 'f') {
      if (fields.length < 3) throw new Error(`OBJ line ${lineNumber}: face needs at least three vertices`);
      const face = fields.map((field) => {
        const [v, vt, vn] = field.split('/');
        return {
          v: resolveIndex(v, vertices.length, lineNumber, 'vertex')!,
          vt: resolveIndex(vt, texcoords.length, lineNumber, 'texture coordinate'),
          vn: resolveIndex(vn, normals.length, lineNumber, 'normal')
        };
      });
      for (let corner = 1; corner < face.length - 1; corner++) {
        triangles.push([face[0]!, face[corner]!, face[corner + 1]!]);
        if (triangles.length > MAX_TRIANGLES) throw new Error('OBJ has too many triangles');
      }
    }
  }

  if (!vertices.length) throw new Error('OBJ does not contain any vertices');
  if (!triangles.length) throw new Error('OBJ does not contain any faces');

  const min: V3 = [Infinity, Infinity, Infinity], max: V3 = [-Infinity, -Infinity, -Infinity];
  for (const vertex of vertices) {
    min[0] = Math.min(min[0], vertex[0]); min[1] = Math.min(min[1], vertex[1]); min[2] = Math.min(min[2], vertex[2]);
    max[0] = Math.max(max[0], vertex[0]); max[1] = Math.max(max[1], vertex[1]); max[2] = Math.max(max[2], vertex[2]);
  }
  const center: V3 = [(min[0] + max[0]) / 2, (min[1] + max[1]) / 2, (min[2] + max[2]) / 2];
  let radius = 0;
  for (const vertex of vertices) radius = Math.max(radius, Math.hypot(vertex[0] - center[0], vertex[1] - center[1], vertex[2] - center[2]));
  if (!(radius > 1e-12)) throw new Error('OBJ geometry has no measurable size');

  const outPositions = new Float32Array(triangles.length * 9);
  const outNormals = new Float32Array(triangles.length * 9);
  const outTexcoords = new Float32Array(triangles.length * 6);
  let positionAt = 0, normalAt = 0, texcoordAt = 0;
  for (const triangle of triangles) {
    const a = vertices[triangle[0]!.v]!, b = vertices[triangle[1]!.v]!, c = vertices[triangle[2]!.v]!;
    const fallbackNormal = faceNormal(a, b, c);
    for (const ref of triangle) {
      const vertex = vertices[ref.v]!;
      outPositions[positionAt++] = (vertex[0] - center[0]) / radius;
      outPositions[positionAt++] = (vertex[1] - center[1]) / radius;
      outPositions[positionAt++] = (vertex[2] - center[2]) / radius;
      const normal = ref.vn == null ? fallbackNormal : normals[ref.vn]!;
      outNormals[normalAt++] = normal[0]; outNormals[normalAt++] = normal[1]; outNormals[normalAt++] = normal[2];
      const uv = ref.vt == null ? null : texcoords[ref.vt]!;
      outTexcoords[texcoordAt++] = uv?.[0] ?? 0; outTexcoords[texcoordAt++] = uv?.[1] ?? 0;
    }
  }

  return {
    positions: outPositions,
    normals: outNormals,
    texcoords: outTexcoords,
    vertexCount: triangles.length * 3,
    triangleCount: triangles.length,
    sourceVertexCount: vertices.length,
    bounds: { min, max, center, radius }
  };
}
