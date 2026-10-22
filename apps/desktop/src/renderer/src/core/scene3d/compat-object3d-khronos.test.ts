/* Real glTF import compatibility. GLTFLoader is the test-only reference;
 * production's engine-on path never calls it. Build pm-wasm first. */
import { existsSync, readFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import * as THREE from 'three';
import { imports, PmWasm } from '../../engine-doc/wasm';
import { object3dFromEngine } from './compat-object3d';
import { loadGltf, disposeModel } from './import-model';

const MODULE = fileURLToPath(new URL('../../../public/engine/pm_wasm.wasm', import.meta.url));
const KHRONOS = join(homedir(), 'Library/Application Support/Powermove Engine Corpus/khronos-gltf');
const names = [
  'BoxAnimated.glb', 'RiggedSimple.glb', 'RiggedFigure.glb', 'BrainStem.glb',
  'RecursiveSkeletons.glb', 'AnimatedMorphCube.glb',
  'SimpleInstancing.glb'
].filter(name => existsSync(join(KHRONOS, name)));
const NATIVE_DTO = process.env.PM_NATIVE_IMPORT_DTO_DIR;

function objects(root: THREE.Object3D): THREE.Object3D[] {
  const out: THREE.Object3D[] = [];
  root.traverse(o => out.push(o));
  return out;
}
function numbers(a: ArrayLike<number> | null | undefined): number[] | null {
  return a == null ? null : Array.from(a, Number);
}
function close(a: ArrayLike<number> | null | undefined, b: ArrayLike<number> | null | undefined, label: string, tolerance = 1e-5) {
  const x = numbers(a), y = numbers(b);
  expect(x?.length ?? null, `${label} length`).toBe(y?.length ?? null);
  if (!x || !y) return;
  for (let i = 0; i < x.length; i++) expect(Math.abs(x[i]! - y[i]!), `${label}[${i}]`).toBeLessThanOrEqual(tolerance);
}
function compareGraph(expected: THREE.Object3D, actual: THREE.Object3D, label: string) {
  const a = objects(expected), b = objects(actual);
  expect(b.length, `${label} object count`).toBe(a.length);
  for (let i = 0; i < a.length; i++) {
    const left = a[i]!, right = b[i]!, name = `${label} object ${i}`;
    expect(right.type, `${name} type`).toBe(left.type);
    expect(right.name, `${name} name`).toBe(left.name);
    expect(right.userData, `${name} userData`).toEqual(left.userData);
    expect(right.parent ? b.indexOf(right.parent) : -1, `${name} parent`).toBe(left.parent ? a.indexOf(left.parent) : -1);
    close(left.matrix.elements, right.matrix.elements, `${name} local matrix`, 1e-8);
    if ((left as THREE.Mesh).isMesh) {
      const lg = (left as THREE.Mesh).geometry, rg = (right as THREE.Mesh).geometry;
      expect(rg.userData, `${name} geometry userData`).toEqual(lg.userData);
      for (const key of new Set([...Object.keys(lg.attributes), ...Object.keys(rg.attributes)])) {
        const la = lg.getAttribute(key), ra = rg.getAttribute(key);
        expect(ra?.itemSize, `${name} ${key} itemSize`).toBe(la?.itemSize);
        expect(ra?.count, `${name} ${key} count`).toBe(la?.count);
        if (la && ra) for (let vertex = 0; vertex < la.count; vertex++)
          for (let component = 0; component < la.itemSize; component++) {
            const a = la.getComponent(vertex, component), b = ra.getComponent(vertex, component);
            expect(Number.isNaN(a) ? Number.isNaN(b) : Math.abs(a - b) <= 1e-5,
              `${name} ${key}[${vertex},${component}]`).toBe(true);
          }
      }
      close(lg.index?.array, rg.index?.array, `${name} indices`, 0);
      close((left as THREE.Mesh).morphTargetInfluences, (right as THREE.Mesh).morphTargetInfluences, `${name} morph`);
      expect((right as THREE.Mesh).morphTargetDictionary, `${name} morph names`).toEqual((left as THREE.Mesh).morphTargetDictionary);
      const lm = (left as THREE.Mesh).material as THREE.MeshStandardMaterial;
      const rm = (right as THREE.Mesh).material as THREE.MeshStandardMaterial;
      expect(rm.type, `${name} material type`).toBe(lm.type);
      expect(rm.userData, `${name} material userData`).toEqual(lm.userData);
      expect(rm.name, `${name} material name`).toBe(lm.name);
      expect(rm.side, `${name} side`).toBe(lm.side);
      expect(rm.transparent, `${name} transparent`).toBe(lm.transparent);
      expect(rm.opacity, `${name} opacity`).toBe(lm.opacity);
      expect(rm.alphaTest, `${name} alphaTest`).toBe(lm.alphaTest);
      expect(rm.color.toArray(), `${name} exact color`).toEqual(lm.color.toArray());
      if (lm.emissive) expect(rm.emissive?.toArray(), `${name} exact emissive`).toEqual(lm.emissive.toArray());
      if ('roughness' in lm) expect(rm.roughness, `${name} roughness`).toBe(lm.roughness);
      if ('metalness' in lm) expect(rm.metalness, `${name} metalness`).toBe(lm.metalness);
      if (lm.normalScale) close(lm.normalScale.toArray(), rm.normalScale?.toArray(), `${name} normal scale`);
      if (lm instanceof THREE.MeshPhysicalMaterial && rm instanceof THREE.MeshPhysicalMaterial)
        close(lm.clearcoatNormalScale.toArray(), rm.clearcoatNormalScale.toArray(), `${name} clearcoat normal scale`);
      if (left instanceof THREE.SkinnedMesh && right instanceof THREE.SkinnedMesh) {
        expect(right.skeleton.bones.map(x => x.name), `${name} bones`).toEqual(left.skeleton.bones.map(x => x.name));
        right.skeleton.boneInverses.forEach((m, j) => close(left.skeleton.boneInverses[j]!.elements, m.elements, `${name} inverse ${j}`, 1e-7));
      }
      if (left instanceof THREE.InstancedMesh && right instanceof THREE.InstancedMesh) {
        expect(right.count).toBe(left.count);
        close(left.instanceMatrix.array, right.instanceMatrix.array, `${name} instances`, 0);
      }
    }
  }
  const sharing = (nodes: THREE.Object3D[], property: 'geometry' | 'material') => {
    const first = new Map<object, number>();
    return nodes.map(node => {
      const value = (node as any)[property] as object | undefined;
      if (!value) return -1;
      if (!first.has(value)) first.set(value, first.size);
      return first.get(value)!;
    });
  };
  expect(sharing(b, 'geometry'), `${label} geometry identity partition`).toEqual(sharing(a, 'geometry'));
  expect(sharing(b, 'material'), `${label} material identity partition`).toEqual(sharing(a, 'material'));
}
function compareClips(reference: THREE.AnimationClip[], actual: THREE.AnimationClip[], label: string) {
  expect(actual.length, `${label} clip count`).toBe(reference.length);
  reference.forEach((clip, i) => {
    const other = actual[i]!;
    expect(other.name, `${label} clip name`).toBe(clip.name);
    expect(other.userData, `${label} clip userData`).toEqual(clip.userData);
    expect(other.duration, `${label} duration`).toBe(clip.duration);
    expect(other.tracks.length, `${label} track count`).toBe(clip.tracks.length);
    clip.tracks.forEach((track, j) => {
      const got = other.tracks[j]!;
      // UUIDs on unnamed nodes are instance-specific; compare the property.
      expect(got.name.split('.').at(-1), `${label} track property`).toBe(track.name.split('.').at(-1));
      close(track.times, got.times, `${label} clip ${i} track ${j} times`, 0);
      close(track.values, got.values, `${label} clip ${i} track ${j} values`, 1e-6);
      for (const t of [0, clip.duration * .25, clip.duration * .5, clip.duration * .75])
        close((track as any).createInterpolant().evaluate(t), (got as any).createInterpolant().evaluate(t), `${label} clip ${i} track ${j} t=${t}`, 2e-5);
    });
  });
}

describe.skipIf((!existsSync(MODULE) && !NATIVE_DTO) || !names.length)('Khronos engine Object3D compatibility', () => {
  it.each(names)('%s graph, materials, skin, morph, clips and pose', async name => {
    const bytes = readFileSync(join(KHRONOS, name));
    const data = NATIVE_DTO
      ? JSON.parse(readFileSync(join(NATIVE_DTO, name + '.json'), 'utf8'))
      : await (async () => {
          const instance = await WebAssembly.instantiate(await WebAssembly.compile(readFileSync(MODULE)), imports);
          const engine = new PmWasm(instance);
          if (!engine.x.pm_model_import) throw new Error('Rebuild pm-wasm before this comparison');
          return engine.importModel(bytes);
        })();
    const actual = await object3dFromEngine(data, bytes);
    const reference = await loadGltf(new Blob([bytes], { type: 'model/gltf-binary' }));
    try {
      expect(actual.vertices).toBe(reference.vertices);
      expect(actual.triangles).toBe(reference.triangles);
      compareGraph(reference.object3d, actual.object3d, name);
      const expectedBounds = new THREE.Box3().setFromObject(reference.object3d, true);
      const actualBounds = new THREE.Box3().setFromObject(actual.object3d, true);
      close(expectedBounds.min.toArray(), actualBounds.min.toArray(), `${name} bounds minimum`, 2e-5);
      close(expectedBounds.max.toArray(), actualBounds.max.toArray(), `${name} bounds maximum`, 2e-5);
      compareClips(reference.animations, actual.animations, name);
      for (const clipIndex of reference.animations.map((_, i) => i)) {
        // Tracks targeting unnamed nodes use UUIDs. Animate the imported
        // roots themselves so these bindings exercise the real adapter.
        const expected = reference.object3d, got = actual.object3d;
        const em = new THREE.AnimationMixer(expected), gm = new THREE.AnimationMixer(got);
        em.clipAction(reference.animations[clipIndex]!).play();
        gm.clipAction(actual.animations[clipIndex]!).play();
        for (const fraction of [0, .25, .5, .75]) {
          const time = reference.animations[clipIndex]!.duration * fraction;
          em.setTime(time); gm.setTime(time);
          expected.updateMatrixWorld(true); got.updateMatrixWorld(true);
          const eo = objects(expected), go = objects(got);
          eo.forEach((o, i) => {
            close(o.matrixWorld.elements, go[i]!.matrixWorld.elements, `${name} pose ${clipIndex}/${fraction} object ${i}`, 2e-5);
            close((o as THREE.Mesh).morphTargetInfluences, (go[i] as THREE.Mesh).morphTargetInfluences, `${name} pose ${clipIndex}/${fraction} morph ${i}`, 2e-5);
            if (o instanceof THREE.SkinnedMesh && go[i] instanceof THREE.SkinnedMesh) {
              o.skeleton.update(); go[i].skeleton.update();
              close(o.skeleton.boneMatrices, go[i].skeleton.boneMatrices, `${name} pose ${clipIndex}/${fraction} bones ${i}`, 2e-5);
            }
          });
        }
        em.stopAllAction(); gm.stopAllAction();
      }
    } finally { disposeModel(actual.object3d); disposeModel(reference.object3d); }
  }, 30_000);
});
