/** Displace source geometry once; every placed copy keeps the same BLAS. */
import * as THREE from 'three';
import type { StudioMesh, StudioVec3 } from '../scene3d/studio-backend';
export type StudioDisplacement = {
  object: number;
  width: number;
  height: number;
  values: number[];
  strength: number;
  mid_level?: number;
  subdivision?: number;
};
const vector = (p: StudioVec3) => new THREE.Vector3(p.x, p.y, p.z);
const point = (p: THREE.Vector3): StudioVec3 => ({ x: p.x, y: p.y, z: p.z });
export function displaceMesh(source: StudioMesh, d: StudioDisplacement): StudioMesh {
  const level = d.subdivision ?? 0,
    mid = d.mid_level ?? 0.5;
  if (
    !Number.isInteger(d.width) ||
    !Number.isInteger(d.height) ||
    d.width < 1 ||
    d.height < 1 ||
    d.width > 4096 ||
    d.height > 4096 ||
    d.values.length !== d.width * d.height ||
    d.values.some((v) => !Number.isFinite(v) || v < 0 || v > 1) ||
    !Number.isFinite(d.strength) ||
    !Number.isFinite(mid) ||
    mid < 0 ||
    mid > 1 ||
    !Number.isInteger(level) ||
    level < 0 ||
    level > 5 ||
    source.indices.length * 4 ** level > 1000000
  )
    throw new Error('Invalid displacement settings or triangle budget');
  const positions = source.positions.map(vector),
    normals = source.normals.map(vector),
    uvs = source.uvs.map((uv) => [...uv] as [number, number]);
  let indices = source.indices.map((t) => [...t] as [number, number, number]);
  for (let iteration = 0; iteration < level; iteration++) {
    const edges = new Map<string, number>(),
      next: [number, number, number][] = [];
    const midpoint = (a: number, b: number) => {
      const key = a < b ? `${a}:${b}` : `${b}:${a}`;
      const previous = edges.get(key);
      if (previous !== undefined) return previous;
      const index = positions.length;
      positions.push(positions[a]!.clone().add(positions[b]!).multiplyScalar(0.5));
      normals.push(normals[a]!.clone().add(normals[b]!).normalize());
      uvs.push([(uvs[a]![0] + uvs[b]![0]) * 0.5, (uvs[a]![1] + uvs[b]![1]) * 0.5]);
      edges.set(key, index);
      return index;
    };
    for (const [a, b, c] of indices) {
      const ab = midpoint(a, b),
        bc = midpoint(b, c),
        ca = midpoint(c, a);
      next.push([a, ab, ca], [ab, b, bc], [ca, bc, c], [ab, bc, ca]);
    }
    indices = next;
  }
  const height = (uv: [number, number]) => {
    const x = Math.max(0, Math.min(1, uv[0])) * (d.width - 1),
      y = Math.max(0, Math.min(1, uv[1])) * (d.height - 1),
      ix = Math.floor(x),
      iy = Math.floor(y),
      u = x - ix,
      v = y - iy;
    const at = (a: number, b: number) => d.values[Math.min(d.height - 1, b) * d.width + Math.min(d.width - 1, a)]!;
    return (
      (at(ix, iy) * (1 - u) + at(ix + 1, iy) * u) * (1 - v) + (at(ix, iy + 1) * (1 - u) + at(ix + 1, iy + 1) * u) * v
    );
  };
  positions.forEach((p, i) => p.addScaledVector(normals[i]!.clone().normalize(), (height(uvs[i]!) - mid) * d.strength));
  const reference = normals.map((n) => n.clone());
  normals.forEach((n) => n.set(0, 0, 0));
  for (const triangle of indices) {
    const [a, b, c] = triangle,
      p = positions[a]!,
      q = positions[b]!,
      r = positions[c]!,
      n = q.clone().sub(p).cross(r.clone().sub(p)),
      expected = reference[a]!.clone().add(reference[b]!).add(reference[c]!);
    if (n.dot(expected) < 0) n.negate();
    triangle.forEach((i) => normals[i]!.add(n));
  }
  normals.forEach((n, i) => (n.length() > 1e-10 ? n.normalize() : n.copy(reference[i]!)));
  return { positions: positions.map(point), normals: normals.map(point), uvs, indices };
}
