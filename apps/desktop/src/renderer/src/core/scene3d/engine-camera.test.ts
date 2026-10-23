import { describe, expect, it } from 'vitest';
import * as THREE from 'three';
import { LAYER3D_DEFINITIONS } from './layers';
import { editScene } from './operations';
import { engineCamera, viewportSnapshot } from './commands';
import { setViewportMode, orbitViewStep, toggleViewProjection, enterLocalView } from './viewport';
import { makePM } from '../../legacy/__tests__/make-pm';

function editor() {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
  PM.proj = PM.mkProject({ name: '3D', fps: 30, dur: 5 }); PM.time = 0;
  PM.layerDefinition = (id: string) => Object.values(LAYER3D_DEFINITIONS).includes(id as any) ? { id, label: '3D', version: 1, params: [], defaults: {}, renderer: { kind: 'layer3d' } } : null;
  editScene(PM, { operation: 'add_object', object: { id: 'a', source: { primitive: 'box' } } });
  editScene(PM, { operation: 'add_object', object: { id: 'b', source: { primitive: 'sphere' }, p: { x: 2 } } });
  return PM;
}

/** The engine's reading of contract 2's camera (`View3d::from_override`): world = view⁻¹, ortho zoom = 3 / half height. */
function engineProjection(camera: NonNullable<ReturnType<typeof engineCamera>>, aspect: number): THREE.Matrix4 {
  const p = camera.projection;
  const three = p.kind === 'perspective'
    ? new THREE.PerspectiveCamera(p.fovYDegrees, aspect, p.near, p.far)
    : Object.assign(new THREE.OrthographicCamera(-3 * aspect, 3 * aspect, 3, -3, p.near, p.far), { zoom: 3 / p.halfHeight });
  three.updateProjectionMatrix();
  return three.projectionMatrix;
}

describe('the editor camera for engine renders', () => {
  it('is absent in the camera view and follows the editor view', () => {
    const PM = editor();
    expect(engineCamera(PM)).toBeNull();
    expect(viewportSnapshot(PM).engineCamera).toBeNull();
    setViewportMode(PM, 'editor');
    orbitViewStep(PM, 30, 15);
    const camera = engineCamera(PM)!;
    expect(camera.projection.kind).toBe('perspective');
    const world = new THREE.Matrix4().fromArray(camera.view).invert();
    const expected = new THREE.Matrix4();
    // The same pose the legacy compositor renders the editor view with.
    const snapshot = viewportSnapshot(PM);
    expect(snapshot.engineCamera).toEqual(camera);
    expected.copy(world);
    expect(world.elements.every(Number.isFinite)).toBe(true);
    // Projection the engine rebuilds equals the editor camera's at the same aspect.
    for (const aspect of [16 / 9, 1, 0.5]) {
      const legacy = new THREE.PerspectiveCamera(snapshot.view.fov, aspect, (camera.projection as any).near, (camera.projection as any).far);
      legacy.updateProjectionMatrix();
      expect(engineProjection(camera, aspect).elements).toEqual(legacy.projectionMatrix.elements);
    }
    toggleViewProjection(PM);
    const ortho = engineCamera(PM)!;
    expect(ortho.projection.kind).toBe('orthographic');
    expect(enterLocalView(PM, ['a'])).toBe(true);
    expect(viewportSnapshot(PM).localView).toEqual(['a']);
  });
});
