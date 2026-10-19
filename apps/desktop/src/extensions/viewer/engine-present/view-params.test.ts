import { describe, expect, it } from 'vitest';

import { previewResolution, viewParamsFor } from './view-params';
import { ViewSequencer } from './view-state';

describe('view parameters', () => {
  it('maps the preview resolution menu', () => {
    expect(previewResolution('auto')).toBe('automatic');
    expect(previewResolution(null)).toBe('automatic');
    expect(previewResolution('1')).toBe('full');
    expect(previewResolution(0.5)).toBe('half');
    expect(previewResolution('0.25')).toBe('quarter');
  });

  it('sends pan only out of fit mode and the 3D override only for 3D compositions', () => {
    expect(viewParamsFor({ shown: 0.4, fit: true, pan: [30, 4], previewResolution: 'auto' })).toEqual({
      zoom: 0.4,
      pan: [0, 0],
      resolution: 'automatic',
      media: 'automatic',
      playEveryFrame: false
    });
    const flat = viewParamsFor({ shown: 2, fit: false, pan: [30, 4], previewResolution: 1, scene3d: { hasScene: false, shading: 'solid', xray: true } });
    expect(flat).not.toHaveProperty('shading');
    expect(flat.pan).toEqual([30, 4]);
    const scene = viewParamsFor({ shown: 1, fit: true, pan: [0, 0], previewResolution: 'auto', scene3d: { hasScene: true, shading: 'solid', xray: true } });
    expect(scene).toMatchObject({ camera: null, shading: 'solid', xray: true });
  });

  it('yields a view override with X-ray only in draft shading', () => {
    const sequencer = new ViewSequencer();
    const base = { composition: 'c', cssWidth: 100, cssHeight: 50, devicePixelRatio: 2, colorSpace: 'srgb' as const, edr: false };
    const solid = sequencer.next({ ...base, ...viewParamsFor({ shown: 1, fit: true, pan: [0, 0], previewResolution: 'auto', scene3d: { hasScene: true, shading: 'solid', xray: true } }) });
    expect(solid.state.view).toEqual({ camera: null, shading: 'solid', xray: true, localView: [] });
    const material = sequencer.next({ ...base, ...viewParamsFor({ shown: 1, fit: true, pan: [0, 0], previewResolution: 'auto', scene3d: { hasScene: true, shading: 'material', xray: true } }) });
    expect(material.state.view).toEqual({ camera: null, shading: 'material', xray: false, localView: [] });
    expect(material.state.viewSeq).toBe(2);
  });

  it('carries the editor camera and Local View of the 3D viewport', () => {
    const sequencer = new ViewSequencer();
    const base = { composition: 'c', cssWidth: 100, cssHeight: 50, devicePixelRatio: 2, colorSpace: 'srgb' as const, edr: false };
    const camera = { view: Array.from({ length: 16 }, (_, i) => (i % 5 === 0 ? 1 : 0)), projection: { kind: 'perspective' as const, fovYDegrees: 50, near: 0.01, far: 1000 } };
    const editor = sequencer.next({ ...base, ...viewParamsFor({ shown: 1, fit: true, pan: [0, 0], previewResolution: 'auto', scene3d: { hasScene: true, shading: 'solid', xray: false, camera, localView: ['a', 'b'] } }) });
    expect(editor.state.view).toEqual({ camera, shading: 'solid', xray: false, localView: ['a', 'b'] });
    const back = sequencer.next({ ...base, ...viewParamsFor({ shown: 1, fit: true, pan: [0, 0], previewResolution: 'auto', scene3d: { hasScene: true, shading: 'solid', xray: false, camera: null, localView: null } }) });
    expect(back.state.view).toEqual({ camera: null, shading: 'solid', xray: false, localView: [] });
    expect(back.changed).toBe(true);
  });
});
