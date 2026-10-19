import { describe, expect, it } from 'vitest';

import { canvasColorSpace, deviceSize, displayIsEdr, ViewSequencer, type ViewInput } from './view-state';

const input: ViewInput = {
  composition: 'main',
  cssWidth: 800,
  cssHeight: 450,
  devicePixelRatio: 2,
  zoom: 0.25,
  pan: [0, 0],
  resolution: 'automatic',
  media: 'automatic',
  colorSpace: 'srgb',
  edr: false,
  playEveryFrame: false
};

describe('ViewSequencer', () => {
  it('numbers changes and keeps equal states', () => {
    const sequencer = new ViewSequencer();
    const first = sequencer.next(input);
    expect(first.changed).toBe(true);
    expect(first.state.viewSeq).toBe(1);
    expect(first.state.size).toEqual({ width: 1600, height: 900 });
    expect(first.state.view).toBeNull();
    const same = sequencer.next({ ...input });
    expect(same).toEqual({ state: first.state, changed: false });
    const panned = sequencer.next({ ...input, pan: [3, 4] });
    expect(panned.changed).toBe(true);
    expect(panned.state.viewSeq).toBe(2);
  });

  it('carries the view override, X-ray only in draft shading', () => {
    const sequencer = new ViewSequencer();
    const camera = { view: Array.from({ length: 16 }, (_, i) => (i % 5 === 0 ? 1 : 0)), projection: { kind: 'perspective' as const, fovYDegrees: 50, near: 0.1, far: 100 } };
    const rendered = sequencer.next({ ...input, camera, shading: 'rendered', xray: true }).state;
    expect(rendered.view).toEqual({ camera, shading: 'rendered', xray: false, localView: [] });
    const solid = sequencer.next({ ...input, camera, shading: 'solid', xray: true, localView: ['a'] }).state;
    expect(solid.view).toEqual({ camera, shading: 'solid', xray: true, localView: ['a'] });
    // Rendered shading without a camera is still an override: the engine
    // draws Blender surfaces in it while paused.
    const blender = sequencer.next({ ...input, shading: 'rendered' }).state;
    expect(blender.view).toEqual({ camera: null, shading: 'rendered', xray: false, localView: [] });
    expect(sequencer.next({ ...input }).state.view).toBeNull();
  });

  it('carries today’s canvas; a change of it is a new view', () => {
    const canvas = { width: 600, height: 338, crop: { x: 0, y: 0, width: 600, height: 338 }, css: { x: 10, y: 20, width: 366, height: 206.18 } };
    const sequencer = new ViewSequencer();
    const none = sequencer.next(input).state;
    expect(none).not.toHaveProperty('canvas');
    const withCanvas = sequencer.next({ ...input, canvas });
    expect(withCanvas.changed).toBe(true);
    expect(withCanvas.state.canvas).toEqual(canvas);
    expect(withCanvas.state.viewSeq).toBe(none.viewSeq + 1);
    expect(sequencer.next({ ...input, canvas: { ...canvas, crop: { ...canvas.crop } } }).changed).toBe(false);
    expect(sequencer.next({ ...input, canvas: { ...canvas, width: 300 } }).changed).toBe(true);
    // A canvas the engine would refuse is not sent (it draws at zoom × DPR).
    for (const bad of [{ ...canvas, width: 0 }, { ...canvas, height: 1.5 }, { ...canvas, crop: { ...canvas.crop, width: 0 } }, { ...canvas, css: { ...canvas.css, x: Number.NaN } }]) {
      expect(new ViewSequencer().next({ ...input, canvas: bad }).state).not.toHaveProperty('canvas');
    }
  });

  it('sanitizes sizes and numbers', () => {
    expect(deviceSize(0, 10_000, 2)).toEqual({ width: 1, height: 16384 });
    const state = new ViewSequencer().next({ ...input, zoom: Number.NaN, devicePixelRatio: 0, pan: [Number.POSITIVE_INFINITY, 2] }).state;
    expect([state.zoom, state.devicePixelRatio, state.pan]).toEqual([1, 1, [0, 2]]);
  });
});

describe('display queries', () => {
  it('reads EDR and wide gamut from media queries', () => {
    const host = (matches: string[]) => ({ matchMedia: (q: string) => ({ matches: matches.includes(q) }) });
    expect(displayIsEdr(host(['(dynamic-range: high)']))).toBe(true);
    expect(displayIsEdr(host([]))).toBe(false);
    expect(canvasColorSpace(host(['(color-gamut: p3)']))).toBe('displayP3');
    expect(canvasColorSpace({})).toBe('srgb');
  });
});
