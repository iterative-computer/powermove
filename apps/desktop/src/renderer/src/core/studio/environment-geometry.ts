import * as THREE from 'three';
import type { StudioMesh, StudioSceneDescription, StudioVec3 } from '../scene3d/studio-backend';
export type StudioInstance = {
  object: number;
  position?: StudioVec3;
  rotation?: StudioVec3;
  scale?: StudioVec3;
  layer?: number | null;
};
export type StudioTerrain = Partial<{
  center: StudioVec3;
  size: [number, number];
  resolution: number;
  amplitude: number;
  frequency: number;
  octaves: number;
  seed: number;
  ridged: boolean;
  heights: number[];
  material: number;
  layers: { material: number; min_height: number; max_height: number; min_slope: number; max_slope: number }[];
}>;
export type StudioScatter = Partial<{
  objects: number[];
  seed: number;
  count: number;
  density: number;
  terrain: number | null;
  surface: number | null;
  center: StudioVec3;
  size: [number, number];
  scale: [number, number];
  rotation: [number, number];
  min_height: number;
  max_height: number;
  max_slope: number;
  exclusions: { center: StudioVec3; radius: number }[];
}>;
const zero = { x: 0, y: 0, z: 0 },
  v = (p: StudioVec3) => new THREE.Vector3(p.x, p.y, p.z),
  to = (p: THREE.Vector3): StudioVec3 => ({ x: p.x, y: p.y, z: p.z }),
  clamp = (x: number, a: number, b: number) => Math.max(a, Math.min(b, x));
export function instanceMatrix(i: StudioInstance): THREE.Matrix4 {
  const r = i.rotation ?? zero,
    rotation = new THREE.Quaternion().setFromEuler(
      new THREE.Euler((r.x * Math.PI) / 180, (r.y * Math.PI) / 180, (r.z * Math.PI) / 180, 'ZYX'),
    );
  return new THREE.Matrix4().compose(v(i.position ?? zero), rotation, v(i.scale ?? { x: 1, y: 1, z: 1 }));
}
function hash(x: number) {
  const s = (Math.imul(x, 747796405) + 2891336453) >>> 0,
    w = Math.imul((s >>> ((s >>> 28) + 4)) ^ s, 277803737) >>> 0;
  return ((w >>> 22) ^ w) >>> 0;
}
function noise(x: number, z: number, seed: number) {
  const a = Math.floor(x),
    b = Math.floor(z),
    fx = x - a,
    fz = z - b,
    u = fx * fx * (3 - 2 * fx),
    w = fz * fz * (3 - 2 * fz);
  const at = (i: number, j: number) =>
    (hash(Math.imul(i, 1597334677) ^ Math.imul(j, 3812015801) ^ seed) / 4294967295) * 2 - 1;
  return (at(a, b) * (1 - u) + at(a + 1, b) * u) * (1 - w) + (at(a, b + 1) * (1 - u) + at(a + 1, b + 1) * u) * w;
}
function terrainSettings(input: StudioTerrain) {
  return {
    center: zero,
    size: [100, 100] as [number, number],
    resolution: 64,
    amplitude: 10,
    frequency: 0.02,
    octaves: 5,
    seed: 1,
    ridged: false,
    heights: [] as number[],
    material: 0,
    layers: [] as NonNullable<StudioTerrain['layers']>,
    ...input,
  };
}
function gridHeight(t: ReturnType<typeof terrainSettings>, x: number, z: number) {
  if (t.heights.length) return t.center.y + t.heights[z * (t.resolution + 1) + x]! * t.amplitude;
  const px = (x / t.resolution - 0.5) * t.size[0] + t.center.x,
    pz = (z / t.resolution - 0.5) * t.size[1] + t.center.z;
  let value = 0,
    norm = 0,
    amp = 1,
    freq = t.frequency;
  for (let o = 0; o < t.octaves; o++) {
    const n = noise(px * freq, pz * freq, (t.seed + o) >>> 0);
    value += amp * (t.ridged ? 1 - Math.abs(n) : n);
    norm += amp;
    amp *= 0.5;
    freq *= 2;
  }
  return t.center.y + (value / norm) * t.amplitude;
}
export function sampleTerrain(input: StudioTerrain, x: number, z: number): { height: number; normal: THREE.Vector3 } {
  const t = terrainSettings(input),
    fx = clamp((x - t.center.x) / t.size[0] + 0.5, 0, 1) * t.resolution,
    fz = clamp((z - t.center.z) / t.size[1] + 0.5, 0, 1) * t.resolution,
    ix = Math.min(t.resolution - 1, Math.floor(fx)),
    iz = Math.min(t.resolution - 1, Math.floor(fz)),
    u = fx - ix,
    w = fz - iz;
  const a = gridHeight(t, ix, iz),
    b = gridHeight(t, ix + 1, iz),
    c = gridHeight(t, ix, iz + 1),
    d = gridHeight(t, ix + 1, iz + 1);
  const [height, dx, dz] =
    w <= u ? [a + (b - a) * u + (d - b) * w, b - a, d - b] : [a + (d - c) * u + (c - a) * w, d - c, c - a];
  return {
    height: height!,
    normal: new THREE.Vector3((-dx! * t.resolution) / t.size[0], 1, (-dz! * t.resolution) / t.size[1]).normalize(),
  };
}
function terrainMeshes(input: StudioTerrain): { mesh: StudioMesh; material: number }[] {
  const t = terrainSettings(input),
    n = t.resolution;
  if (
    !Number.isInteger(n) ||
    n < 2 ||
    n > 512 ||
    t.octaves < 1 ||
    t.octaves > 12 ||
    t.size.some((v) => !Number.isFinite(v) || v <= 0) ||
    t.frequency <= 0 ||
    !Number.isFinite(t.amplitude)
  )
    throw new Error('Invalid terrain settings');
  if (t.heights.length && t.heights.length !== (n + 1) ** 2) throw new Error('Invalid terrain heightfield dimensions');
  const positions: StudioVec3[] = [],
    normals: StudioVec3[] = [],
    uvs: [number, number][] = [];
  for (let z = 0; z <= n; z++)
    for (let x = 0; x <= n; x++) {
      const px = t.center.x + (x / n - 0.5) * t.size[0],
        pz = t.center.z + (z / n - 0.5) * t.size[1],
        xm = Math.max(0, x - 1),
        xp = Math.min(n, x + 1),
        zm = Math.max(0, z - 1),
        zp = Math.min(n, z + 1);
      const dx = (gridHeight(t, xp, z) - gridHeight(t, xm, z)) / (((xp - xm) * t.size[0]) / n),
        dz = (gridHeight(t, x, zp) - gridHeight(t, x, zm)) / (((zp - zm) * t.size[1]) / n);
      positions.push({ x: px, y: gridHeight(t, x, z), z: pz });
      normals.push(to(new THREE.Vector3(-dx, 1, -dz).normalize()));
      uvs.push([x / n, z / n]);
    }
  const groups = new Map<number, [number, number, number][]>();
  for (let z = 0; z < n; z++)
    for (let x = 0; x < n; x++) {
      const a = z * (n + 1) + x,
        b = a + 1,
        c = a + n + 1,
        d = c + 1;
      for (const tri of [
        [a, d, b],
        [a, c, d],
      ] as [number, number, number][]) {
        const [p, q, r] = tri.map((i) => v(positions[i]!)),
          center = p!
            .clone()
            .add(q!)
            .add(r!)
            .multiplyScalar(1 / 3),
          normal = q!.clone().sub(p!).cross(r!.clone().sub(p!)).normalize(),
          slope = (Math.acos(clamp(normal.y, -1, 1)) * 180) / Math.PI;
        const material =
          t.layers.find(
            (l) => center.y >= l.min_height && center.y <= l.max_height && slope >= l.min_slope && slope <= l.max_slope,
          )?.material ?? t.material;
        const indices = groups.get(material) ?? [];
        indices.push(tri);
        groups.set(material, indices);
      }
    }
  return [...groups]
    .sort(([a], [b]) => a - b)
    .map(([material, indices]) => ({ material, mesh: { positions, normals, uvs, indices } }));
}
export function generateScatter(input: StudioScatter, scene: StudioSceneDescription['scene']): StudioInstance[] {
  const s = {
    objects: [0],
    seed: 1,
    count: 100,
    density: 1,
    terrain: null as number | null,
    surface: null as number | null,
    center: zero,
    size: [100, 100] as [number, number],
    scale: [0.8, 1.2] as [number, number],
    rotation: [0, 360] as [number, number],
    min_height: -1e9,
    max_height: 1e9,
    max_slope: 60,
    exclusions: [] as NonNullable<StudioScatter['exclusions']>,
    ...input,
  };
  if (
    !Number.isInteger(s.count) ||
    s.count < 0 ||
    s.count > 1000000 ||
    s.objects.length === 0 ||
    s.objects.some((i) => !scene.objects[i]) ||
    s.density < 0 ||
    s.density > 1 ||
    s.scale[0] <= 0 ||
    s.scale[1] < s.scale[0] ||
    (s.terrain !== null && s.surface !== null)
  )
    throw new Error('Invalid scatter settings');
  let state = s.seed >>> 0;
  const rnd = () => {
    state = hash(state);
    return (state >>> 8) / 16777216;
  };
  const terrain = s.terrain === null ? null : scene.terrains?.[s.terrain],
    surface = s.surface === null ? null : scene.objects[s.surface]?.mesh;
  if ((s.terrain !== null && !terrain) || (s.surface !== null && !surface)) throw new Error('Invalid scatter source');
  const cdf: number[] = [];
  let total = 0;
  if (surface)
    for (const t of surface.indices) {
      const [a, b, c] = t.map((i) => v(surface.positions[i]!));
      total += b!.sub(a!).cross(c!.sub(a!)).length() * 0.5;
      cdf.push(total);
    }
  const out: StudioInstance[] = [];
  for (let i = 0; i < s.count; i++) {
    const r = [rnd(), rnd(), rnd(), rnd(), rnd(), rnd()];
    if (r[0]! >= s.density) continue;
    let p: THREE.Vector3, n: THREE.Vector3;
    if (surface) {
      if (total <= 0) continue;
      const target = r[1]! * total;
      let lo = 0,
        hi = cdf.length - 1;
      while (lo < hi) {
        const mid = (lo + hi) >>> 1;
        if (cdf[mid]! < target) lo = mid + 1;
        else hi = mid;
      }
      const [a, b, c] = surface.indices[lo]!.map((i) => v(surface.positions[i]!)),
        u = Math.sqrt(r[2]!),
        w = r[3]!;
      p = a!
        .clone()
        .multiplyScalar(1 - u)
        .addScaledVector(b!, u * (1 - w))
        .addScaledVector(c!, u * w);
      n = b!.clone().sub(a!).cross(c!.clone().sub(a!)).normalize();
    } else {
      const x = s.center.x + (r[1]! - 0.5) * s.size[0],
        z = s.center.z + (r[2]! - 0.5) * s.size[1],
        sample = terrain ? sampleTerrain(terrain, x, z) : { height: s.center.y, normal: new THREE.Vector3(0, 1, 0) };
      p = new THREE.Vector3(x, sample.height, z);
      n = sample.normal;
    }
    if (
      p.y < s.min_height ||
      p.y > s.max_height ||
      (Math.acos(clamp(n.y, -1, 1)) * 180) / Math.PI > s.max_slope ||
      s.exclusions.some((e) => (p.x - e.center.x) ** 2 + (p.z - e.center.z) ** 2 < e.radius ** 2)
    )
      continue;
    const scale = s.scale[0] + (s.scale[1] - s.scale[0]) * r[4]!;
    out.push({
      object: s.objects[Math.min(s.objects.length - 1, Math.floor(r[5]! * s.objects.length))]!,
      position: to(p),
      rotation: { x: 0, y: s.rotation[0] + (s.rotation[1] - s.rotation[0]) * r[3]!, z: 0 },
      scale: { x: scale, y: scale, z: scale },
    });
  }
  return out;
}
export function expandEnvironmentGeometry(document: StudioSceneDescription): StudioSceneDescription {
  const source = document.scene;
  if (!source.terrains?.length && !source.scatters?.length) return document;
  const objects = [...source.objects],
    layers = source.layers?.slice() ?? source.objects.map((_, i) => i),
    ids = [...document.object_ids],
    layerIds = [...document.layer_ids];
  const instances = source.instances?.length
    ? [...source.instances]
    : objects.map((_, object) => ({ object }) as StudioInstance);
  for (const scatter of source.scatters ?? [])
    for (const instance of generateScatter(scatter, source)) instances.push(instance);
  for (const terrain of source.terrains ?? [])
    for (const object of terrainMeshes(terrain)) {
      const index = objects.length,
        layer = layerIds.length;
      objects.push(object);
      layers.push(layer);
      instances.push({ object: index });
      layerIds.push(`terrain-${index}`);
      ids.push({ layer: `terrain-${index}`, part: null, cast_shadow: true, receive_shadow: true });
    }
  if (instances.length > 1000000) throw new Error('At most one million Studio instances');
  return {
    ...document,
    object_ids: ids,
    layer_ids: layerIds,
    scene: { ...source, objects, layers, instances, scatters: [], terrains: [] },
  };
}
