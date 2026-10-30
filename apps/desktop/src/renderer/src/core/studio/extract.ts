import * as THREE from 'three';
import { buildBvh } from './bvh';

/*
 * Turns the three.js graph the scene runtime already maintains (primitives,
 * lathes, extrusions, OBJ/glTF imports, their materials and maps, lights and
 * the camera) into the flat buffers trace.wgsl reads. Each mesh keeps a BVH
 * in its own space; a small top-level BVH over their world bounds is rebuilt
 * when anything moves (two-level ray tracing).
 */

export const MAP_SIZE = 1024;

/** An image map, or a live screen: a composition rendered each frame. */
export type StudioMap = { source: TexImageSource; flipY: boolean } | { screen: string };

export type StudioScene = {
  positions: Float32Array;   // 12 floats per triangle (3 × vec4), model space, per-mesh BVH order
  tris: Float32Array;        // 24 floats per triangle (6 × vec4)
  /** Wide nodes: the top level (tlasNodes, fixed capacity) then every mesh's own BVH. */
  tlasNodes: ArrayBuffer;
  blasNodes: ArrayBuffer;
  /** Identifies positions/tris/blasNodes: equal keys mean only the top level and instances moved. */
  geometryKey: string;
  /** Lights (16 floats each) then instances (36 floats each), as trace.wgsl's scene data. */
  sdata: Float32Array;
  instanceCount: number;
  materials: Float32Array;   // 24 floats per material
  lightCount: number;
  maps: StudioMap[];         // texture-array layers
  /** Scene object id (layer id) of each object index the tracer reports. */
  objects: string[];
  triangles: number;
  /** World-space bounds, for the shadow catcher and epsilon scale. */
  bounds: THREE.Box3;
};

export type EnvironmentInput = { sky: THREE.Color; ground: THREE.Color; intensity: number };

const tmp = new THREE.Vector3(), tmpN = new THREE.Vector3();

function materialKey(m: any): string {
  return m.uuid;
}

/** Collects the meshes of `objects` (ids) under `scene`, skipping hidden ones. */
function meshesOf(scene: THREE.Scene, include: (id: string) => boolean): { mesh: THREE.Mesh; id: string }[] {
  const out: { mesh: THREE.Mesh; id: string }[] = [];
  const visit = (o: THREE.Object3D, visible: boolean) => {
    const v = visible && o.visible;
    if (!v) return;
    const mesh = o as THREE.Mesh;
    const id = o.userData?.sceneId;
    if ((mesh as any).isMesh && !(mesh as any).isLine && id && include(id) && !o.userData?.missing) out.push({ mesh, id });
    for (const c of o.children) visit(c, v);
  };
  visit(scene, true);
  return out;
}

export function extractScene(scene: THREE.Scene, include: (id: string) => boolean, environment: EnvironmentInput): StudioScene {
  scene.updateMatrixWorld(true);
  const meshes = meshesOf(scene, include);
  const objects: string[] = [];
  const objectIndex = new Map<string, number>();
  const materials: number[] = [];
  const materialIndex = new Map<string, number>();
  const maps: StudioMap[] = [];
  const mapIndex = new Map<any, number>();
  const bounds = new THREE.Box3();

  const layer = (texture: THREE.Texture | null | undefined): number => {
    const image: any = texture?.image;
    if (!image || !(image.width || image.videoWidth)) return -1;
    let i = mapIndex.get(image);
    if (i === undefined) {
      i = maps.length;
      maps.push({ source: image, flipY: texture!.flipY });
      mapIndex.set(image, i);
    }
    return i;
  };

  const material = (m: any): number => {
    const key = materialKey(m);
    let i = materialIndex.get(key);
    if (i !== undefined) return i;
    i = materialIndex.size;
    materialIndex.set(key, i);
    const color = m.color || new THREE.Color(1, 1, 1);
    const screen: string | null = m.userData?.screen ?? null;
    // A screen glows with its composition: white emission scaled by the strength.
    const emissive = screen ? new THREE.Color(1, 1, 1).multiplyScalar(Math.max(m.emissiveIntensity ?? 1, 0)) : (m.emissive || new THREE.Color(0, 0, 0)).clone().multiplyScalar(m.emissiveIntensity ?? 1);
    let screenLayer = -1;
    if (screen) {
      const key = `screen:${screen}`;
      screenLayer = mapIndex.get(key) ?? -1;
      if (screenLayer < 0) { screenLayer = maps.length; maps.push({ screen }); mapIndex.set(key, screenLayer); }
    }
    const basic = m.isMeshBasicMaterial;
    const metallic = basic ? 0 : m.metalness ?? 0;
    const roughness = basic ? 1 : m.roughness ?? 1;
    const transmission = m.transmission ?? 0;
    const opacity = m.transparent || m.opacity < 1 ? m.opacity ?? 1 : 1;
    materials.push(
      color.r, color.g, color.b, metallic,
      roughness, m.clearcoat ?? 0, m.clearcoatRoughness ?? 0.03, transmission,
      // A basic (unlit) material glows its own color.
      // emit.w: subsurface radius; extra.w: anisotropy.
      basic ? color.r : emissive.r, basic ? color.g : emissive.g, basic ? color.b : emissive.b, m.userData?.subsurface ?? 0,
      m.ior ?? 1.5, m.specularIntensity ?? 1, opacity, m.userData?.anisotropy ?? 0,
      basic ? -1 : layer(m.map), screen ? screenLayer : layer(basic ? m.map : m.emissiveMap), layer(m.roughnessMap || m.metalnessMap), layer(m.normalMap),
      // Studio's material layers: sheen, thin film (nm), dispersion, droplets.
      m.sheen ?? 0, m.iridescence > 0 ? (m.iridescenceThicknessRange?.[1] ?? 400) : 0, m.userData?.studioDispersion ?? 0, m.userData?.droplets ?? 0,
    );
    return i;
  };

  const tangent = (p: number[], uv: number[]): number[] => {
    const at = (a: number[], i: number) => a[i] ?? 0;
    const e1 = [0, 1, 2].map(a => at(p, 3 + a) - at(p, a)), e2 = [0, 1, 2].map(a => at(p, 6 + a) - at(p, a));
    const d1 = [at(uv, 2) - at(uv, 0), at(uv, 3) - at(uv, 1)], d2 = [at(uv, 4) - at(uv, 0), at(uv, 5) - at(uv, 1)];
    const r = at(d1, 0) * at(d2, 1) - at(d2, 0) * at(d1, 1);
    if (Math.abs(r) < 1e-12) return [0, 0, 0, 1];
    const f = 1 / r;
    const t = [0, 1, 2].map(a => f * (at(d2, 1) * at(e1, a) - at(d1, 1) * at(e2, a)));
    const b = [0, 1, 2].map(a => f * (at(d1, 0) * at(e2, a) - at(d2, 0) * at(e1, a)));
    const cross = (x: number[], y: number[]) => [at(x, 1) * at(y, 2) - at(x, 2) * at(y, 1), at(x, 2) * at(y, 0) - at(x, 0) * at(y, 2), at(x, 0) * at(y, 1) - at(x, 1) * at(y, 0)];
    const tb = cross(cross(e1, e2), t);
    const handed = at(tb, 0) * at(b, 0) + at(tb, 1) * at(b, 1) + at(tb, 2) * at(b, 2) < 0 ? -1 : 1;
    const len = Math.hypot(at(t, 0), at(t, 1), at(t, 2)) || 1;
    return [at(t, 0) / len, at(t, 1) / len, at(t, 2) / len, handed];
  };

  // One BVH per mesh, in its own space, reused until its geometry or material
  // assignment changes; only the top level is rebuilt when things move.
  const blases: Blas[] = [];
  const instances: { blas: Blas; world: THREE.Matrix4 }[] = [];
  for (const { mesh, id } of meshes) {
    const g = mesh.geometry as THREE.BufferGeometry;
    const posAttr = g.getAttribute('position');
    if (!posAttr) continue;
    let oi = objectIndex.get(id);
    if (oi === undefined) { oi = objects.length; objects.push(id); objectIndex.set(id, oi); }
    const mats = Array.isArray(mesh.material) ? mesh.material : [mesh.material];
    const mi = mats.map((m: any) => (!m || m.visible === false || (m.opacity ?? 1) <= 0 ? -1 : material(m)));
    const key = blasKey(g, mi, oi);
    let blas = blasCache.get(mesh);
    if (!blas || blas.key !== key) {
      blas = buildBlas(mesh, g, mi, oi, key, tangent);
      blasCache.set(mesh, blas);
    }
    if (!blas.triangles) continue;
    blases.push(blas);
    instances.push({ blas, world: mesh.matrixWorld.clone() });
    bounds.union(blas.bounds.clone().applyMatrix4(mesh.matrixWorld));
  }
  // Geometry in layout order (stable across frames while meshes do not change).
  const unique = [...new Set(blases)];
  const geometryKey = unique.map(b => b.key).join('|');
  const tlasCapacity = Math.max(1, instances.length + 1);
  let geometry = geometryCache.get(geometryKey);
  if (!geometry) {
    const triangles = unique.reduce((n, b) => n + b.triangles, 0);
    const nodeCount = unique.reduce((n, b) => n + b.nodes.byteLength / 64, 0);
    const positions = new Float32Array(Math.max(1, triangles) * 12);
    const tris = new Float32Array(Math.max(1, triangles) * 24);
    const blasNodes = new ArrayBuffer(Math.max(1, nodeCount) * 64);
    const nf = new Float32Array(blasNodes), nu = new Uint32Array(blasNodes);
    const roots = new Map<Blas, number>();
    let triOffset = 0, nodeOffset = 0;
    for (const b of unique) {
      positions.set(b.positions, triOffset * 12);
      tris.set(b.tris, triOffset * 24);
      const src = new Uint32Array(b.nodes), count = b.nodes.byteLength / 64;
      nu.set(src, nodeOffset * 16);
      // Re-base child links: interior → absolute node (after the top level), leaf → absolute triangle.
      for (let n = 0; n < count; n++) for (const side of [0, 8]) {
        const o = (nodeOffset + n) * 16 + side, c = nu[o + 7]!;
        if (c === 0xffffffff) continue;
        nu[o + 3] = c === 0 ? nu[o + 3]! + tlasCapacity + nodeOffset : nu[o + 3]! + triOffset;
      }
      roots.set(b, tlasCapacity + nodeOffset);
      triOffset += b.triangles; nodeOffset += count;
    }
    void nf;
    geometry = { positions, tris, blasNodes, roots, triangles, tlasCapacity };
    geometryCache.clear();
    geometryCache.set(geometryKey, geometry);
  }

  // Top level over the instances' world bounds (a box as a degenerate triangle).
  const instanceBoxes = new Float32Array(instances.length * 9);
  instances.forEach(({ blas, world }, i) => {
    const b = blas.bounds.clone().applyMatrix4(world);
    instanceBoxes.set([b.min.x, b.min.y, b.min.z, b.max.x, b.max.y, b.max.z, b.min.x, b.min.y, b.min.z], i * 9);
  });
  const top = instances.length ? buildBvh(instanceBoxes) : { nodes: new ArrayBuffer(64), order: new Uint32Array(0) };
  const tlasNodes = new ArrayBuffer(geometry.tlasCapacity * 64);
  new Uint8Array(tlasNodes).set(new Uint8Array(top.nodes).subarray(0, Math.min(top.nodes.byteLength, tlasNodes.byteLength)));

  // Lights.
  const lights: number[] = [];
  const size = bounds.isEmpty() ? 1 : bounds.getSize(tmp).length();
  scene.traverseVisible((o: any) => {
    if (!o.isLight || o.isHemisphereLight || o.isAmbientLight) return;
    const c = o.color.clone().multiplyScalar(o.intensity);
    const position = new THREE.Vector3().setFromMatrixPosition(o.matrixWorld);
    if (o.isDirectionalLight) {
      const target = new THREE.Vector3().setFromMatrixPosition(o.target.matrixWorld);
      const dir = position.sub(target).normalize();
      // A soft sun (1.5°) reads as studio light rather than a hard noon.
      lights.push(dir.x, dir.y, dir.z, 3, Math.cos(THREE.MathUtils.degToRad(1.5)), 0, 0, 0, 0, 0, 0, 0, c.r, c.g, c.b, 0);
    } else if (o.isRectAreaLight) {
      const q = new THREE.Quaternion().setFromRotationMatrix(o.matrixWorld);
      const u = new THREE.Vector3(o.width / 2, 0, 0).applyQuaternion(q), v = new THREE.Vector3(0, o.height / 2, 0).applyQuaternion(q);
      // three's rect lights emit along -z: order u × v accordingly.
      lights.push(position.x, position.y, position.z, 0, v.x, v.y, v.z, o.width * o.height, u.x, u.y, u.z, 0, c.r, c.g, c.b, 0);
    } else if (o.isPointLight || o.isSpotLight) {
      const r = Math.max(size * 0.004, 1e-3);
      const radiance = c.multiplyScalar(1 / (Math.PI * r * r));
      if (o.isSpotLight) {
        const target = new THREE.Vector3().setFromMatrixPosition(o.target.matrixWorld);
        const axis = target.sub(position).normalize();
        const outer = Math.cos(o.angle), inner = Math.cos(o.angle * (1 - (o.penumbra ?? 0)));
        // The cone test compares the ray toward the light with the axis, so flip it.
        lights.push(position.x, position.y, position.z, 2, r, 0, 0, 0, -axis.x, -axis.y, -axis.z, outer, radiance.r, radiance.g, radiance.b, Math.max(inner, outer + 1e-4));
      } else {
        lights.push(position.x, position.y, position.z, 1, r, 0, 0, 0, 0, 0, 0, 0, radiance.r, radiance.g, radiance.b, 0);
      }
    }
  });

  // Scene data: lights, then instances in top-level order (9 vec4 each).
  const sdata = new Float32Array(Math.max(4, lights.length + instances.length * 36));
  sdata.set(lights, 0);
  const nm = new THREE.Matrix3(), inv = new THREE.Matrix4();
  for (let k = 0; k < instances.length; k++) {
    const { blas, world } = instances[top.order[k]!]!;
    const o = lights.length + k * 36, w = world.elements, iv = inv.copy(world).invert().elements, n = nm.getNormalMatrix(world).elements;
    for (let r = 0; r < 3; r++) {
      sdata.set([iv[r]!, iv[4 + r]!, iv[8 + r]!, iv[12 + r]!], o + r * 4);
      sdata.set([n[r]!, n[3 + r]!, n[6 + r]!, 0], o + 12 + r * 4);
      sdata.set([w[r]!, w[4 + r]!, w[8 + r]!, 0], o + 24 + r * 4);
    }
    sdata[o + 15] = geometry.roots.get(blas)!;
  }

  void environment;
  return {
    positions: geometry.positions, tris: geometry.tris, blasNodes: geometry.blasNodes, tlasNodes, geometryKey,
    sdata, instanceCount: instances.length,
    materials: new Float32Array(materials.length ? materials : new Array(24).fill(0)),
    lightCount: lights.length / 16,
    maps, objects, triangles: geometry.triangles, bounds,
  };
}

type Blas = { key: string; positions: Float32Array; tris: Float32Array; nodes: ArrayBuffer; bounds: THREE.Box3; triangles: number };
const blasCache = new WeakMap<THREE.Object3D, Blas>();
const geometryCache = new Map<string, { positions: Float32Array; tris: Float32Array; blasNodes: ArrayBuffer; roots: Map<Blas, number>; triangles: number; tlasCapacity: number }>();
const versions = new WeakMap<object, number>();
let nextVersion = 1;
/** A stable identity for attribute contents (three bumps `.version` on edits). */
function idOf(o: any): string {
  if (!o) return '-';
  let v = versions.get(o);
  if (!v) { v = nextVersion++; versions.set(o, v); }
  return `${v}.${o.version ?? 0}`;
}
function blasKey(g: THREE.BufferGeometry, materials: number[], object: number): string {
  return [g.uuid, idOf(g.getAttribute('position')), idOf(g.getAttribute('normal')), idOf(g.getAttribute('uv')), idOf(g.getIndex()), materials.join(','), object, g.groups.map(x => `${x.start}:${x.count}:${x.materialIndex}`).join(';')].join('/');
}

function buildBlas(mesh: THREE.Mesh, g: THREE.BufferGeometry, mi: number[], oi: number, key: string, tangent: (p: number[], uv: number[]) => number[]): Blas {
  const posAttr = g.getAttribute('position'), nrm = g.getAttribute('normal'), uvA = g.getAttribute('uv');
  const index = g.getIndex();
  const groups = Array.isArray(mesh.material) && g.groups.length ? g.groups : [{ start: 0, count: index ? index.count : posAttr.count, materialIndex: 0 }];
  const corners: number[] = [], shading: number[] = [];
  const bounds = new THREE.Box3();
  for (const group of groups) {
    const m = mi[group.materialIndex ?? 0] ?? -1;
    if (m < 0) continue;
    const end = Math.min(group.start + group.count, index ? index.count : posAttr.count);
    for (let k = group.start; k + 2 < end; k += 3) {
      const vi = [0, 1, 2].map(j => (index ? index.getX(k + j) : k + j));
      const p: number[] = [], n: number[] = [], uv: number[] = [];
      for (const v of vi) {
        tmp.fromBufferAttribute(posAttr as any, v);
        p.push(tmp.x, tmp.y, tmp.z);
        bounds.expandByPoint(tmp);
        if (nrm) tmpN.fromBufferAttribute(nrm as any, v).normalize(); else tmpN.set(0, 0, 0);
        n.push(tmpN.x, tmpN.y, tmpN.z);
        uv.push(uvA ? uvA.getX(v) : 0, uvA ? uvA.getY(v) : 0);
      }
      if (!nrm) {
        // Faceted normals.
        const v = (i: number) => p[i] ?? 0;
        const ax = v(3) - v(0), ay = v(4) - v(1), az = v(5) - v(2), bx = v(6) - v(0), by = v(7) - v(1), bz = v(8) - v(2);
        const nx = ay * bz - az * by, ny = az * bx - ax * bz, nz = ax * by - ay * bx, l = Math.hypot(nx, ny, nz) || 1;
        for (let j = 0; j < 3; j++) { n[j * 3] = nx / l; n[j * 3 + 1] = ny / l; n[j * 3 + 2] = nz / l; }
      }
      corners.push(...p);
      // Tangents drive normal maps and brushed (anisotropic) highlights.
      const t = uvA ? tangent(p, uv) : [0, 0, 0, 0];
      const nn = (i: number) => n[i] ?? 0, uu = (i: number) => uv[i] ?? 0;
      shading.push(
        nn(0), nn(1), nn(2), 0, nn(3), nn(4), nn(5), 0, nn(6), nn(7), nn(8), 0,
        uu(0), uu(1), uu(2), uu(3),
        uu(4), uu(5), m, oi,
        ...t,
      );
    }
  }
  const triangles = corners.length / 9;
  const cornerArray = new Float32Array(corners);
  const { nodes, order } = triangles ? buildBvh(cornerArray) : { nodes: new ArrayBuffer(64), order: new Uint32Array(0) };
  const positions = new Float32Array(Math.max(1, triangles) * 12);
  const tris = new Float32Array(Math.max(1, triangles) * 24);
  for (let i = 0; i < triangles; i++) {
    const t = order[i]!;
    for (let c = 0; c < 3; c++) {
      positions[i * 12 + c * 4] = cornerArray[t * 9 + c * 3]!;
      positions[i * 12 + c * 4 + 1] = cornerArray[t * 9 + c * 3 + 1]!;
      positions[i * 12 + c * 4 + 2] = cornerArray[t * 9 + c * 3 + 2]!;
    }
    tris.set(shading.slice(t * 24, t * 24 + 24), i * 24);
  }
  return { key, positions: positions.subarray(0, triangles * 12), tris: tris.subarray(0, triangles * 24), nodes, bounds, triangles };
}
