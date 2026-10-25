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

  const camera = { view: Array.from({ length: 16 }, (_, i) => (i % 5 === 0 ? 1 : 0)), projection: { kind: 'perspective' as const, fovYDegrees: 40, near: 0.1, far: 100 } };

  it('sends pan only out of fit mode and the 3D override only in the Editor view', () => {
    expect(viewParamsFor({ shown: 0.4, fit: true, pan: [30, 4], previewResolution: 'auto' })).toEqual({
      zoom: 0.4,
      pan: [0, 0],
      resolution: 'automatic',
      media: 'automatic',
      playEveryFrame: false
    });
    const flat = viewParamsFor({ shown: 2, fit: false, pan: [30, 4], previewResolution: 1, editorCamera: null });
    expect(flat).not.toHaveProperty('camera');
    expect(flat).not.toHaveProperty('shading');
    expect(flat.pan).toEqual([30, 4]);
    const editor = viewParamsFor({ shown: 1, fit: true, pan: [0, 0], previewResolution: 'auto', editorCamera: camera });
    expect(editor).toMatchObject({ camera });
    expect(editor).not.toHaveProperty('shading');
    expect(editor).not.toHaveProperty('xray');
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

  it('yields a view override with the editor camera and full shading, and none in the Camera view', () => {
    const sequencer = new ViewSequencer();
    const base = { composition: 'c', cssWidth: 100, cssHeight: 50, devicePixelRatio: 2, colorSpace: 'srgb' as const, edr: false };
    const editor = sequencer.next({ ...base, ...viewParamsFor({ shown: 1, fit: true, pan: [0, 0], previewResolution: 'auto', editorCamera: camera }) });
    expect(editor.state.view).toEqual({ camera, shading: 'rendered', xray: false, localView: [] });
    const composition = sequencer.next({ ...base, ...viewParamsFor({ shown: 1, fit: true, pan: [0, 0], previewResolution: 'auto', editorCamera: null }) });
    expect(composition.state.view).toBeNull();
    expect(composition.state.viewSeq).toBe(2);
  });

  it('carries a changed editor projection and clears the override in Camera view', () => {
    const sequencer = new ViewSequencer();
    const base = { composition: 'c', cssWidth: 100, cssHeight: 50, devicePixelRatio: 2, colorSpace: 'srgb' as const, edr: false };
    const camera = { view: Array.from({ length: 16 }, (_, i) => (i % 5 === 0 ? 1 : 0)), projection: { kind: 'perspective' as const, fovYDegrees: 50, near: 0.01, far: 1000 } };
    const canvas = todaysCanvas(600, 338, { width: 600, height: 338 }, null, { x: -9, y: 60.5 }, 0.61)!;
    const editor = sequencer.next({ ...base, ...viewParamsFor({ shown: 1, fit: true, pan: [0, 0], previewResolution: 'auto', canvas, editorCamera: camera }) });
    // The SVG canvas contract and the 3D editor view must coexist.
    expect(editor.state.canvas).toEqual(canvas);
    expect(editor.state.view).toEqual({ camera, shading: 'rendered', xray: false, localView: [] });
    const changedCamera = { ...camera, projection: { ...camera.projection, fovYDegrees: 65 } };
    const changed = sequencer.next({ ...base, ...viewParamsFor({ shown: 1, fit: true, pan: [0, 0], previewResolution: 'auto', editorCamera: changedCamera }) });
    expect(changed.state.view?.camera).toEqual(changedCamera);
    expect(changed.changed).toBe(true);
    const back = sequencer.next({ ...base, ...viewParamsFor({ shown: 1, fit: true, pan: [0, 0], previewResolution: 'auto', editorCamera: null }) });
    expect(back.state.view).toBeNull();
    expect(back.changed).toBe(true);
  });
});
