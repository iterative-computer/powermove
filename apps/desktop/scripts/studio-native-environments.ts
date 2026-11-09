/** Deterministic geometry stress scenes for the native transport benchmark.
 * These are simple fixtures, not claims of production environment quality. */
import * as THREE from 'three';
import { extractScene } from '../src/renderer/src/core/studio/extract';
import type { StudioView } from '../src/renderer/src/core/studio/gpu';

export function nativeEnvironment(name: string) {
  const graph = new THREE.Scene();
  const geometries = new Set<THREE.BufferGeometry>();
  const materials = new Set<THREE.Material>();
  let object = 0;
  const material = (color: number, roughness = .7, metalness = 0) => {
    const m = new THREE.MeshStandardMaterial({ color, roughness, metalness, side: THREE.DoubleSide });
    materials.add(m); return m;
  };
  const add = (g: THREE.BufferGeometry, m: THREE.Material, p: number[], scale = [1, 1, 1]) => {
    geometries.add(g);
    const mesh = new THREE.Mesh(g, m); mesh.position.fromArray(p); mesh.scale.fromArray(scale);
    mesh.castShadow = mesh.receiveShadow = true;
    mesh.userData.sceneId = `environment-${object++}`; mesh.userData.studioPart = 'surface';
    graph.add(mesh); return mesh;
  };
  const box = new THREE.BoxGeometry(1, 1, 1);
  const wood = material(0x995c35), white = material(0xe7e3dc), dark = material(0x26313a);
  let eye: THREE.Vector3, target: THREE.Vector3;
  if (name === 'room' || name === 'room-ortho') {
    add(box, wood, [0, -.15, 0], [12, .3, 10]);
    add(box, white, [0, 2.5, -5], [12, 5, .2]);
    add(box, white, [-6, 2.5, 0], [.2, 5, 10]);
    add(box, wood, [0, 1.7, 0], [4, .2, 2]);
    for (const x of [-1.7, 1.7]) for (const z of [-.7, .7]) add(box, dark, [x, .8, z], [.12, 1.6, .12]);
    for (let i = 0; i < 12; i++) {
      const m = material(new THREE.Color().setHSL(i / 12, .45, .4).getHex());
      add(box, m, [-1.6 + i * .27, 1.9 + (i % 3) * .025, 0], [.2, .2 + (i % 3) * .05, .6]);
    }
    const globe = material(0xc6d4e0, .12, .9);
    add(new THREE.SphereGeometry(.45, 32, 16), globe, [1.1, 2.25, .5]);
    // A deliberately large direct-light list exercises bounded resampling.
    for (let i = 0; i < 64; i++) {
      const light = new THREE.PointLight(i % 2 ? 0xffddbb : 0xddeeff, .35);
      light.position.set((i % 8 - 3.5) * 1.2, 4.2, (Math.floor(i / 8) - 3.5));
      light.castShadow = true; graph.add(light);
    }
    eye = new THREE.Vector3(8, 5, 10); target = new THREE.Vector3(0, 1.3, 0);
  } else if (name === 'forest') {
    add(box, material(0x526647), [0, -.15, 0], [100, .3, 100]);
    const bark = material(0x65452e), leaves = material(0x284e31);
    const trunk = new THREE.CylinderGeometry(.2, .3, 3, 6), crown = new THREE.ConeGeometry(1.8, 5, 8);
    for (let i = 0; i < 144; i++) {
      const x = (i % 12 - 5.5) * 5 + Math.sin(i * 9.7), z = (Math.floor(i / 12) - 5.5) * 5;
      add(trunk, bark, [x, 1.5, z]); add(crown, leaves, [x, 5, z]);
    }
    const sun = new THREE.DirectionalLight(0xffe8ca, 2.5); sun.position.set(20, 40, 10); sun.castShadow = true; graph.add(sun);
    eye = new THREE.Vector3(34, 18, 40); target = new THREE.Vector3(0, 2, 0);
  } else throw new Error(`Unknown environment fixture: ${name}`);
  graph.updateMatrixWorld(true);
  const scene = extractScene(graph, () => true, { sky: new THREE.Color(), ground: new THREE.Color(), intensity: 0 });
  const forward = target.clone().sub(eye).normalize(), right = forward.clone().cross(new THREE.Vector3(0, 1, 0)).normalize(), up = right.clone().cross(forward);
  const tuple = (v: THREE.Vector3) => v.toArray() as [number, number, number];
  const view: StudioView = { camera: { position: tuple(eye), forward: tuple(forward), right: tuple(right), up: tuple(up), tanY: name.endsWith('-ortho') ? 4 : .4, orthographic: name.endsWith('-ortho'), aspect: 16 / 9, lensRadius: 0, focus: eye.distanceTo(target), near: .01, far: 1000 }, exposure: 1, backdrop: [.65, .72, .8], backdropOpaque: true, shadowCatcher: false, shadows: true, floorY: -1, environment: { zenith: [.35, .45, .65], horizon: [.3, .33, .38], ground: [.08, .08, .08] } };
  return { scene, view, dispose() { geometries.forEach(g => g.dispose()); materials.forEach(m => m.dispose()); } };
}
