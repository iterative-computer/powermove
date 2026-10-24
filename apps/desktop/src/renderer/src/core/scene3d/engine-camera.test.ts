import { describe, expect, it } from 'vitest';
import * as THREE from 'three';
import { LAYER3D_DEFINITIONS } from './layers';
import { editScene } from './operations';
import { orbitView } from './navigation';
import { applyViewportPose, editorCamera, setViewportMode, viewportEngineCamera, viewportPose } from './viewport';
import { makePM } from '../../legacy/__tests__/make-pm';

function editor() {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
  PM.proj = PM.mkProject({ name: '3D', fps: 30, dur: 5 }); PM.time = 0;
  PM.layerDefinition = (id: string) => Object.values(LAYER3D_DEFINITIONS).includes(id as any) ? { id, label: '3D', version: 1, params: [], defaults: {}, renderer: { kind: 'layer3d' } } : null;
  editScene(PM, { operation: 'add_object', object: { id: 'a', source: { primitive: 'box' } } });
  editScene(PM, { operation: 'add_object', object: { id: 'b', source: { primitive: 'sphere' }, p: { x: 2 } } });
  return PM;
}

describe('the editor camera for engine renders', () => {
  it('is absent in Camera view and matches the transient three.js camera after orbit', () => {
    const PM = editor();
    expect(viewportEngineCamera(PM)).toBeNull();
    expect(editorCamera(PM)).toBeNull();
    setViewportMode(PM, 'editor');
    applyViewportPose(PM, orbitView(viewportPose(PM), 30, 15));
    const override = viewportEngineCamera(PM)!;
    const legacy = editorCamera(PM)!;
    expect(override.projection.kind).toBe('perspective');
    new THREE.Matrix4().fromArray(override.view).invert().elements.forEach((value, index) =>
      expect(value).toBeCloseTo(legacy.matrixWorld.elements[index]!, 12));
    if (override.projection.kind !== 'perspective' || !(legacy instanceof THREE.PerspectiveCamera)) throw new Error('Expected a perspective camera');
    const zoom = legacy.zoom > 0 ? legacy.zoom : 1;
    const fov = THREE.MathUtils.radToDeg(2 * Math.atan(Math.tan(THREE.MathUtils.degToRad(legacy.fov) / 2) / zoom));
    expect(override.projection).toEqual({ kind: 'perspective', fovYDegrees: fov, near: legacy.near, far: legacy.far });
    expect(override.view.every(Number.isFinite)).toBe(true);
    setViewportMode(PM, 'camera');
    expect(viewportEngineCamera(PM)).toBeNull();
  });
});
