import * as THREE from 'three';
import { extractScene, MAP_SIZE } from './extract';
import { studioGpuReady, StudioTarget } from './gpu';

/*
 * Finish swatches rendered by Studio itself: each preset on a sphere under
 * two softboxes on the studio backdrop, 32 samples, read back as an image.
 * Rendered once per preset and cached.
 */

const SIZE = 96;
const cache = new Map<string, Promise<string | null>>();
let target: StudioTarget | null = null;

function material(p: Record<string, number | string>): THREE.MeshPhysicalMaterial {
  const m = new THREE.MeshPhysicalMaterial({
    color: String(p.color ?? '#cccccc'), metalness: Number(p.metalness ?? 0), roughness: Number(p.roughness ?? 0.5),
    clearcoat: Number(p.clearcoat ?? 0), clearcoatRoughness: Number(p.clearcoatRoughness ?? 0.03),
    transmission: Number(p.transmission ?? 0), ior: Number(p.ior ?? 1.5), specularIntensity: Number(p.specular ?? 1),
  });
  m.userData.anisotropy = Number(p.anisotropy ?? 0);
  m.userData.subsurface = Number(p.subsurface ?? 0);
  m.sheen = Number(p.sheen ?? 0);
  return m;
}

/** A data URL of `p` rendered on a sphere, or null where Studio cannot run. */
export function studioSwatch(id: string, p: Record<string, number | string>): Promise<string | null> {
  if (!studioGpuReady()) return Promise.resolve(null);
  const key = `${id}:${JSON.stringify(p)}`;
  let pending = cache.get(key);
  if (pending) return pending;
  pending = (async () => {
    const scene = new THREE.Scene();
    const sphere = new THREE.Mesh(new THREE.SphereGeometry(1, 64, 32), material(p));
    sphere.userData.sceneId = 'swatch';
    scene.add(sphere);
    const key = new THREE.RectAreaLight(0xffffff, 6, 3, 2); key.position.set(-2.5, 3, 3); key.lookAt(0, 0, 0);
    const rim = new THREE.RectAreaLight(0xffffff, 4, 4, 0.8); rim.position.set(3, 1, -2.5); rim.lookAt(0, 0, 0);
    scene.add(key, rim);
    const studio = extractScene(scene, () => true, { sky: new THREE.Color(), ground: new THREE.Color(), intensity: 0 });
    target ??= new StudioTarget(SIZE, SIZE);
    // Texture maps are not part of a swatch: one blank layer.
    target.setScene(studio, [new Uint8Array(MAP_SIZE * MAP_SIZE * 4)]);
    const camera = new THREE.PerspectiveCamera(28, 1, 0.1, 100);
    camera.position.set(0, 0.6, 4.4); camera.lookAt(0, 0, 0); camera.updateMatrixWorld(true);
    const e = camera.matrixWorld.elements;
    target.setView({
      camera: {
        position: [e[12]!, e[13]!, e[14]!], forward: [-e[8]!, -e[9]!, -e[10]!], right: [e[0]!, e[1]!, e[2]!], up: [e[4]!, e[5]!, e[6]!],
        tanY: Math.tan(THREE.MathUtils.degToRad(14)), aspect: 1, lensRadius: 0, focus: 4.4,
      },
      exposure: 1, backdrop: [0.965, 0.961, 0.973], backdropOpaque: true, shadowCatcher: false, shadows: true, floorY: 0,
      environment: { zenith: [0.35, 0.35, 0.37], horizon: [0.22, 0.22, 0.24], ground: [0.08, 0.08, 0.09] },
    });
    target.trace(32, 6, true, 64);
    target.resolve(2);
    const gpuCanvas = target.present(`swatch:${id}`, -1, true);
    // Copy out in the same task, before the WebGPU canvas is presented.
    const out = document.createElement('canvas');
    out.width = out.height = SIZE;
    out.getContext('2d')!.drawImage(gpuCanvas, 0, 0);
    await target.done();
    return out.toDataURL('image/png');
  })().catch(() => null);
  cache.set(key, pending);
  return pending;
}
