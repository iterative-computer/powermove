import { engineGeometry } from './engine-access';
/*
 * What Edit Mode and the 3D cursor read of the scene's geometry:
 *
 * - `objectGeometry(PM, id)`: the geometry `runtime.objects.get(id)` holds
 *   (Edit Mode's `currentMesh` for a model without its own mesh), or why
 *   there is none;
 * - `cursorHit(PM, ray)`: `placeCursor`'s surface point, the first visible
 *   mesh hit along the ray.
 *
 * With the native evaluator on, pm-wasm answers both (`objectMesh3d`,
 * `cursorHit3d`) through the engine document's answer path: with "Use New
 * Engine" on that path is strict, so a question pm-wasm cannot answer is a
 * named, visible failure and three.js is never asked (the three.js answers
 * below run only in the development modes' dual answers, as the reference).
 * Off, the composition's three.js runtime answers, as today.
 */
import * as THREE from 'three';
import { compositionRuntime } from './service';
import type { ObjectMesh3d } from '../../engine-doc/native';

const engineOf = (PM: any) => engineGeometry(PM, ['objectMesh3d', 'cursorHit3d']);

/** Today's answer: `runtime.objects.get(id)` and its geometry's buffers. */
export function runtimeObjectMesh(PM: any, id: string, time: number = PM.time): ObjectMesh3d {
  const object = compositionRuntime(PM, time).objects.get(id) as THREE.Mesh | undefined;
  if (!object) return { missing: true };
  if (!object.isMesh) return { notMesh: true };
  const geometry = object.geometry, position = geometry.getAttribute('position'), uv = geometry.getAttribute('uv');
  return {
    positions: position ? Array.from(position.array as ArrayLike<number>) : null,
    ...(uv ? { uvs: Array.from(uv.array as ArrayLike<number>) } : {}),
    index: geometry.index ? Array.from(geometry.index.array as ArrayLike<number>) : null
  };
}

/**
 * The geometry of 3D object layer `id` at `time` as a `BufferGeometry` with
 * today's buffers (Float32 positions and uvs, the index when it has one), or
 * null when the runtime holds no mesh for it.
 */
export function objectGeometry(PM: any, id: string, time: number = PM.time): THREE.BufferGeometry | null {
  const engine = engineOf(PM);
  const answer: ObjectMesh3d = engine?.objectMesh3d
    ? engine.objectMesh3d(id, time, () => runtimeObjectMesh(PM, id, time))
    : runtimeObjectMesh(PM, id, time);
  if ('missing' in answer || 'notMesh' in answer) return null;
  const geometry = new THREE.BufferGeometry();
  // An empty geometry stays empty: reading its position throws as today.
  if (!answer.positions) return geometry;
  geometry.setAttribute('position', new THREE.Float32BufferAttribute(answer.positions, 3));
  if (answer.uvs?.length) geometry.setAttribute('uv', new THREE.Float32BufferAttribute(answer.uvs, 2));
  if (answer.index) geometry.setIndex(answer.index);
  return geometry;
}

/** Today's answer: three.js's raycast over the runtime's objects. */
export function runtimeCursorHit(PM: any, ray: THREE.Ray, time: number = PM.time): number[] | null {
  const world = compositionRuntime(PM, time), caster = new THREE.Raycaster(ray.origin, ray.direction);
  const hit = caster.intersectObjects([...world.objects.values()].filter(o => !(o as any).isLight), true).find(h => h.object.visible && (h.object as any).isMesh);
  return hit ? hit.point.toArray() : null;
}

/** `placeCursor`'s surface point along `ray` at `time`, or null when nothing is hit. */
export function cursorHit(PM: any, ray: THREE.Ray, time: number = PM.time): THREE.Vector3 | null {
  const engine = engineOf(PM);
  const point = engine?.cursorHit3d
    ? engine.cursorHit3d(ray.origin.toArray(), ray.direction.toArray(), time, () => runtimeCursorHit(PM, ray, time))
    : runtimeCursorHit(PM, ray, time);
  return point ? new THREE.Vector3().fromArray(point) : null;
}
