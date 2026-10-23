import { describe, expect, it } from 'vitest';

import { canvasOutputScale, previewResolution, todaysCanvas, viewParamsFor } from './view-params';
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

  it('describes today’s canvas: the whole composition below fit, the crop above', () => {
    // 61% at DPR 2: today's canvas is the composition's size (min(1, 1.22)),
    // in the composition frame's box.
    const whole = todaysCanvas(600, 338, { width: 600, height: 338 }, null, { x: -9, y: 60.5 }, 0.61)!;
    expect(whole).toEqual({ width: 600, height: 338, crop: { x: 0, y: 0, width: 600, height: 338 }, css: { x: -9, y: 60.5, width: 366, height: 206.18 } });
    expect(canvasOutputScale(whole)).toEqual([1, 1]);
    // A crop: its own box inside the frame, its render size over its size.
    const crop = todaysCanvas(600, 338, { width: 1500, height: 900 }, { x: 100, y: 50, width: 150, height: 90 }, { x: -200, y: -100 }, 5)!;
    expect(crop.css).toEqual({ x: 300, y: 150, width: 750, height: 450 });
    expect(canvasOutputScale(crop)).toEqual([10, 10]);
    expect(todaysCanvas(600, 338, { width: Number.NaN, height: 2 }, null, { x: 0, y: 0 }, 1)).toBeNull();
    expect(viewParamsFor({ shown: 0.61, fit: false, pan: [0, 0], previewResolution: 'auto', canvas: whole }).canvas).toEqual(whole);
    expect(viewParamsFor({ shown: 0.61, fit: false, pan: [0, 0], previewResolution: 'auto', canvas: null })).not.toHaveProperty('canvas');
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
