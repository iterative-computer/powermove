import * as THREE from 'three';
import { buildBvh } from './bvh';

/*
 * Turns the three.js graph the scene runtime already maintains (primitives,
 * lathes, extrusions, OBJ/glTF imports, their materials and maps, lights and
 * the camera) into the flat buffers trace.wgsl reads. Geometry is baked to
 * world space; one BVH covers every object of a depth set.
 */

export const MAP_SIZE = 1024;

/** An image map, or a live screen: a composition rendered each frame. */
export type StudioMap = { source: TexImageSource; flipY: boolean } | { screen: string };

export type StudioScene = {
  positions: Float32Array;   // 12 floats per triangle (3 × vec4), BVH order
  tris: Float32Array;        // 24 floats per triangle (6 × vec4), BVH order
  nodes: ArrayBuffer;        // wide BVH nodes
  materials: Float32Array;   // 24 floats per material
  lights: Float32Array;      // 16 floats per light
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
  const corners: number[] = [];
  const shading: number[] = [];
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

  for (const { mesh, id } of meshes) {
    const g = mesh.geometry as THREE.BufferGeometry;
    const posAttr = g.getAttribute('position');
    if (!posAttr) continue;
    let oi = objectIndex.get(id);
    if (oi === undefined) { oi = objects.length; objects.push(id); objectIndex.set(id, oi); }
    const nrm = g.getAttribute('normal'), uvA = g.getAttribute('uv');
    const index = g.getIndex();
    const world = mesh.matrixWorld;
    const normalMatrix = new THREE.Matrix3().getNormalMatrix(world);
    const mats = Array.isArray(mesh.material) ? mesh.material : [mesh.material];
    const groups = Array.isArray(mesh.material) && g.groups.length ? g.groups : [{ start: 0, count: index ? index.count : posAttr.count, materialIndex: 0 }];
    for (const group of groups) {
      const m: any = mats[group.materialIndex ?? 0];
      if (!m || m.visible === false || (m.opacity ?? 1) <= 0) continue;
      const mi = material(m);
      const end = Math.min(group.start + group.count, index ? index.count : posAttr.count);
      for (let k = group.start; k + 2 < end; k += 3) {
        const vi = [0, 1, 2].map(j => (index ? index.getX(k + j) : k + j));
        const p: number[] = [], n: number[] = [], uv: number[] = [];
        for (const v of vi) {
          tmp.fromBufferAttribute(posAttr as any, v).applyMatrix4(world);
          p.push(tmp.x, tmp.y, tmp.z);
          bounds.expandByPoint(tmp);
          if (nrm) tmpN.fromBufferAttribute(nrm as any, v).applyMatrix3(normalMatrix).normalize();
          else tmpN.set(0, 0, 0);
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
          uu(4), uu(5), mi, oi,
          ...t,
        );
      }
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
  if (!triangles) positions.fill(0);

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

  void environment;
  return {
    positions, tris, nodes,
    materials: new Float32Array(materials.length ? materials : new Array(24).fill(0)),
    lights: new Float32Array(lights.length ? lights : new Array(16).fill(0)),
    lightCount: lights.length / 16,
    maps, objects, triangles, bounds,
  };
}
