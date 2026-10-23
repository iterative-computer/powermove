import { describe, expect, it, vi } from 'vitest';
import * as THREE from 'three';
import { viewportGlyphHits } from './glyph-hits';
import { LAYER3D_DEFINITIONS } from './layers';

function scene() {
  const camera = new THREE.OrthographicCamera(-5, 5, 5, -5, 0.1, 100);
  camera.position.z = 10;
  camera.updateMatrixWorld(true);
  const light = { id: 'light', d: { definition: LAYER3D_DEFINITIONS.light } };
  const other = { id: 'other', d: { definition: LAYER3D_DEFINITIONS.light } };
  const view = { id: 'camera', d: { definition: LAYER3D_DEFINITIONS.camera } };
  const object = { id: 'object', d: { definition: LAYER3D_DEFINITIONS.object } };
  let x = 1;
  const native = {
    active: vi.fn((layer: any) => layer.id !== 'other'),
    world3D: vi.fn(() => new THREE.Matrix4().makeTranslation(x, 2, 0).elements)
  };
  const reference = vi.fn(() => { throw new Error('reference geometry must not run'); });
  const PM = { proj: { layers: [light, other, view, object] }, time: 0.25,
    EngineDoc: { enabled: true, api: native }, active: reference, ev: reference };
  return { PM, camera, native, reference, move: (next: number) => { x = next; } };
}

describe('synchronous viewport glyph hit geometry', () => {
  const rect = { x: 0, y: 0, width: 480, height: 270 };

  it('uses current native transforms without waiting for a presented chrome frame', () => {
    const { PM, camera, native, reference, move } = scene();
    const hits = () => viewportGlyphHits(PM, camera, rect, 'camera', null);
    expect(hits()).toEqual([{ id: 'light', x: 288, y: 81 }]);
    move(2);
    expect(hits()).toEqual([{ id: 'light', x: 336, y: 81 }]);
    expect(native.world3D).toHaveBeenCalledWith(PM.proj.layers[0], 0.25);
    expect(reference).not.toHaveBeenCalled();
  });

  it('keeps hidden cameras, inactive lights, objects and local-view exclusions out of hit tests', () => {
    const { PM, camera } = scene();
    expect(viewportGlyphHits(PM, camera, rect, null, new Set(['camera'])))
      .toEqual([{ id: 'camera', x: 288, y: 81 }]);
    expect(viewportGlyphHits(PM, camera, rect, 'camera', new Set(['camera'])))
      .toEqual([]);
  });

  it('fails visibly when the native geometry API is unavailable instead of using the reference', () => {
    const { PM, camera, reference } = scene();
    PM.EngineDoc.api = undefined as any;
    expect(() => viewportGlyphHits(PM, camera, rect, null, null))
      .toThrow('The native 3D geometry API is unavailable (world3D, active)');
    expect(reference).not.toHaveBeenCalled();
  });
});
