import { engineGeometry } from './engine-access';
/*
 * The composition's 3D render camera for the viewport (camera view, framing,
 * the view's field of view, transform gizmos): `compositionRuntime(PM).camera`
 * as today's runtime holds it after `sync`.
 *
 * With the native evaluator on, pm-wasm answers it (`camera3d`, from
 * `compositionScene` like every other 3D question) through the engine
 * document's answer path, so the viewport does not build the three.js
 * runtime for it. With "Use New Engine" on the engine document is strict: a
 * question pm-wasm cannot answer is a named, visible failure, never the
 * runtime's camera (which only the development modes' dual answers read, as
 * the reference). Off, the runtime's camera is returned as today.
 */
import * as THREE from 'three';
import { compositionRuntime } from './service';

/** What the viewport reads of the render camera. */
export type RenderCameraFacts = {
  orthographic: boolean;
  /** Perspective cameras only. */
  fov: number | null;
  zoom: number;
  near: number;
  far: number;
  /** `matrixWorld`, column-major. */
  world: number[];
};

/** The runtime camera's facts (today's answer). */
export function runtimeCameraFacts(PM: any, time: number = PM.time): RenderCameraFacts {
  const camera: any = compositionRuntime(PM, time).camera;
  camera.updateMatrixWorld(true);
  return {
    orthographic: !!camera.isOrthographicCamera,
    fov: camera.isPerspectiveCamera ? camera.fov : null,
    zoom: camera.zoom,
    near: camera.near,
    far: camera.far,
    world: Array.from(camera.matrixWorld.elements as ArrayLike<number>)
  };
}

const cameras = new WeakMap<object, { persp: THREE.PerspectiveCamera; ortho: THREE.OrthographicCamera }>();

/**
 * The render camera at `time`: today's runtime camera with the engine off;
 * with it on, a camera built from pm-wasm's answer (one per PM, reused),
 * its `matrixWorld` exactly the engine's.
 */
export function renderCamera(PM: any, time: number = PM.time): THREE.PerspectiveCamera | THREE.OrthographicCamera {
  const engine = engineGeometry(PM, ['camera3d']);
  if (!engine?.camera3d) return compositionRuntime(PM, time).camera as THREE.PerspectiveCamera | THREE.OrthographicCamera;
  const facts: RenderCameraFacts = engine.camera3d(time, () => runtimeCameraFacts(PM, time));
  let pair = cameras.get(PM);
  if (!pair) {
    pair = { persp: new THREE.PerspectiveCamera(), ortho: new THREE.OrthographicCamera() };
    cameras.set(PM, pair);
  }
  const comp = PM.curComp?.() || PM.proj;
  const aspect = (comp?.w || 16) / Math.max(1, comp?.h || 9);
  const camera = facts.orthographic ? pair.ortho : pair.persp;
  if (camera instanceof THREE.PerspectiveCamera) {
    camera.fov = facts.fov ?? 50;
    camera.aspect = aspect;
  } else {
    camera.left = -3 * aspect; camera.right = 3 * aspect; camera.top = 3; camera.bottom = -3;
  }
  camera.zoom = facts.zoom;
  camera.near = facts.near;
  camera.far = facts.far;
  camera.updateProjectionMatrix();
  // The engine's matrixWorld exactly: no recomposition from position and rotation.
  camera.matrixAutoUpdate = false;
  camera.matrix.fromArray(facts.world);
  camera.matrix.decompose(camera.position, camera.quaternion, camera.scale);
  camera.matrixWorld.copy(camera.matrix);
  camera.matrixWorldInverse.copy(camera.matrixWorld).invert();
  return camera;
}
