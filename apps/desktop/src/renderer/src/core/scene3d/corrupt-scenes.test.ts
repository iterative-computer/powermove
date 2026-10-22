/* Today's behavior for stored 3D data the scene schema refuses (a
 * hand-edited or newer document): the reference for the engine's
 * `pm-render` scene3d rules (docs/engine/scene3d.md, "Corrupted scenes").
 *
 * - A composition's 3D layers: `compositionScene` validates the assembled
 *   scene with `parseScene`, so an unknown primitive or light type on any
 *   layer throws for the whole composition, and the compositor's catch
 *   draws nothing for every object layer (and fails an export).
 * - A legacy scene layer: `sceneRuntime` throws in `sync`, and the
 *   compositor falls through to the missing-definition checkerboard; a
 *   layer whose stored scene is absent draws the same checkerboard. */
import { describe, expect, it } from 'vitest';
import { createScene, SCENE3D_DEFINITION } from './schema';
import { LAYER3D_DEFINITIONS, compositionScene } from './layers';
import { editScene } from './operations';
import { compositionRuntime, sceneRuntime } from './service';
import { makePM } from '../../legacy/__tests__/make-pm';

function editor() {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
  PM.proj = PM.mkProject({ name: '3D', fps: 30, dur: 5 }); PM.time = 0;
  PM.layerDefinition = (id: string) => Object.values(LAYER3D_DEFINITIONS).includes(id as any)
    ? { id, label: '3D', version: 1, params: [], defaults: {}, renderer: { kind: 'layer3d' } }
    : id === SCENE3D_DEFINITION ? { id, label: '3D Scene', version: 1, params: [], defaults: { scene: createScene() }, renderer: { kind: 'scene3d' } } : null;
  return PM;
}

describe('stored 3D data the scene schema refuses', () => {
  it('an unknown primitive on one object layer fails the whole composition scene', () => {
    const PM = editor();
    expect(editScene(PM, { operation: 'add_object', object: { id: 'a', source: { primitive: 'box' } } }).ok).toBe(true);
    expect(editScene(PM, { operation: 'add_object', object: { id: 'b', source: { primitive: 'box' }, p: { x: 2 } } }).ok).toBe(true);
    expect(() => compositionScene(PM, 0)).not.toThrow();
    PM.L('b').d.data.object.source = { primitive: 'knot' };
    expect(() => compositionScene(PM, 0)).toThrow();
    expect(() => compositionRuntime(PM, 0)).toThrow();
  });

  it('an unknown light type on a light layer fails the whole composition scene', () => {
    const PM = editor();
    expect(editScene(PM, { operation: 'add_object', object: { id: 'a', source: { primitive: 'box' } } }).ok).toBe(true);
    expect(editScene(PM, { operation: 'add_light', light: { id: 'l', type: 'point' } }).ok).toBe(true);
    expect(() => compositionScene(PM, 0)).not.toThrow();
    PM.L('l').d.data.light.type = 'laser';
    expect(() => compositionScene(PM, 0)).toThrow();
  });

  it('a legacy scene layer with an unknown primitive or light type throws in sync; one without a scene has none', () => {
    const PM = editor();
    const scene = createScene();
    scene.objects = [{ ...(scene.objects[0] ?? {}), id: 'o', name: 'O', source: { primitive: 'knot' } } as any];
    const layer = { id: 'legacy', type: 'extension', from: 0, dur: 5, on: true, p: {}, d: { definition: SCENE3D_DEFINITION, data: { scene } } };
    expect(() => sceneRuntime(PM, layer, 0)).toThrow();
    const lights = createScene();
    (lights.lights[0] as any).type = 'laser';
    expect(() => sceneRuntime(PM, { ...layer, id: 'legacy-light', d: { definition: SCENE3D_DEFINITION, data: { scene: lights } } }, 0)).toThrow();
    expect(sceneRuntime(PM, { ...layer, id: 'legacy-none', d: { definition: SCENE3D_DEFINITION, data: {} } }, 0)).toBeNull();
  });
});
