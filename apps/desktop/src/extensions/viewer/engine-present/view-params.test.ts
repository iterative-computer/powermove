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

  it('yields a view override with the editor camera and full shading, and none in the Camera view', () => {
    const sequencer = new ViewSequencer();
    const base = { composition: 'c', cssWidth: 100, cssHeight: 50, devicePixelRatio: 2, colorSpace: 'srgb' as const, edr: false };
    const editor = sequencer.next({ ...base, ...viewParamsFor({ shown: 1, fit: true, pan: [0, 0], previewResolution: 'auto', editorCamera: camera }) });
    expect(editor.state.view).toEqual({ camera, shading: 'rendered', xray: false, localView: [] });
    const composition = sequencer.next({ ...base, ...viewParamsFor({ shown: 1, fit: true, pan: [0, 0], previewResolution: 'auto', editorCamera: null }) });
    expect(composition.state.view).toBeNull();
    expect(composition.state.viewSeq).toBe(2);
  });
});
