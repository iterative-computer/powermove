// @vitest-environment happy-dom
import { describe, expect, it, vi } from 'vitest';

import type { RasterGeometry } from '../../../shared/engine-protocol';
import { engineCrop, engineRasterSync, locateLayer, toRasterSurface } from './raster-surface';
import type { EngineRenderTransport, EngineSnapshot } from './render-transport';

const quality = { resolution: 'full', representation: 'original', draft3d: false, cached: false, standIn: false } as const;
const snap = (over: Partial<EngineSnapshot> & { raster: RasterGeometry | null }): EngineSnapshot => ({
  width: 48, height: 28, data: new Uint8ClampedArray(48 * 28 * 4), pixels: 'premultiplied', revision: 9, viewSeq: null,
  origin: [4, 4], layerBox: [48, 28], quality, waitedUs: 0, renderUs: 0, mainMs: 0, ...over
});
const canvas = (s: EngineSnapshot) => Object.assign(document.createElement('canvas'), { width: s.width, height: s.height });

describe('engine rasters in today’s RasterSurface shape', () => {
  it('a shape: box, anchor, selection, and uv for a crop (as rasterShape)', () => {
    const shape = { id: 'b', type: 'solid', d: { w: 40, h: 20 } };
    const whole = toRasterSurface(snap({ raster: { canvas: [48, 28], window: [0, 0], cropped: false, selection: [-20, -10, 20, 10] } }), shape, null, canvas);
    expect(whole).toMatchObject({ w: 48, h: 28, anchorX: 4, anchorY: 4, selection: { x0: -20, y0: -10, x1: 20, y1: 10, w: 40, h: 20 } });
    expect('uv' in whole && whole.uv).toBeUndefined();
    expect(whole.cv.width).toBe(48);
    const crop = { x: 4, y: 4, width: 10, height: 6 };
    const cut = toRasterSurface(snap({ width: 10, height: 6, raster: { canvas: [48, 28], window: [4, 4], cropped: true, selection: [-20, -10, 20, 10] } }), shape, crop, canvas);
    expect(cut.uv).toEqual([-0.4, -4 / 6, 4.8, 28 / 6]);
    expect(cut.cv.width).toBe(10);
  });

  it('plain text with a crop names its source window; text is never blank', () => {
    const text = { id: 't', type: 'text', d: { text: 'Hi' } };
    const surface = toRasterSurface(snap({ width: 20, height: 10, raster: { canvas: [310, 136], window: [5, 6], cropped: true, selection: [-150, -20, 150, 20] } }), text, { x: 5, y: 6, width: 20, height: 10 }, canvas);
    expect(surface.sourceWindow).toEqual([5, 6, 310, 136]);
    expect(surface.blank).toBe(false);
    // Animated text ignores the crop today: the engine says it did not cut.
    const animated = toRasterSurface(snap({ raster: { canvas: [48, 28], window: [0, 0], cropped: false, selection: null } }), text, { x: 5, y: 6, width: 20, height: 10 }, canvas);
    expect(animated.sourceWindow).toBeUndefined();
  });

  it('captions take today’s caption block as their selection', () => {
    const captions = { id: 'c', type: 'captions', d: {} };
    const surface = toRasterSurface(snap({ raster: { canvas: [48, 28], window: [0, 0], cropped: false, selection: null } }), captions, null, canvas, {
      locate: () => ({ composition: null, own: true }),
      captionBounds: () => ({ x0: -100, y0: 300, x1: 100, y1: 340 })
    });
    expect(surface.selection).toEqual({ x0: -100, y0: 300, x1: 100, y1: 340, w: 200, h: 40 });
  });

  it('reads both crop spellings and drops windows that are not', () => {
    expect(engineCrop({ x: 1.4, y: 2, width: 10, height: 5 })).toEqual({ x: 1, y: 2, width: 10, height: 5 });
    expect(engineCrop({ x: 1, y: 2, w: 10, h: 5 })).toEqual({ x: 1, y: 2, width: 10, height: 5 });
    expect(engineCrop({ x: 1 })).toBeNull();
    expect(engineCrop(null)).toBeNull();
  });

  it('sends the caller’s object only when it is not the document’s own layer', () => {
    const own = { id: 'L1', type: 'text', d: { text: 'doc' } };
    const nested = { id: 'N1', type: 'solid', d: { w: 1, h: 1 } };
    const PM = { proj: { id: 'P', compId: 'P-root', layers: [own], comps: { C2: { id: 'C2', layers: [nested] } } }, curComp: () => PM.proj };
    expect(locateLayer(PM, own)).toEqual({ composition: 'P-root', own: true });
    expect(locateLayer(PM, nested)).toEqual({ composition: 'C2', own: true });
    const copy = { ...own, d: { ...own.d, text: 'do' } };
    expect(locateLayer(PM, copy)).toEqual({ composition: 'P-root', own: false });
    const rasterSync = vi.fn(() => snap({ raster: { canvas: [48, 28], window: [0, 0], cropped: false, selection: [0, 0, 1, 1] } }));
    const transport = { rasterSync, toCanvas: canvas } as unknown as EngineRenderTransport;
    const host = { locate: (layer: unknown) => locateLayer(PM, layer) };
    engineRasterSync(transport, host, own, 1, 0.5, null);
    expect(rasterSync).toHaveBeenLastCalledWith('L1', 1, 0.5, null, { composition: 'P-root', content: null });
    engineRasterSync(transport, host, copy, 2, 0.5, { x: 0, y: 0, width: 4, height: 4 });
    const [, , , crop, options] = rasterSync.mock.calls.at(-1) as unknown as [string, number, number, unknown, { content: Uint8Array }];
    expect(crop).toEqual({ x: 0, y: 0, width: 4, height: 4 });
    expect(JSON.parse(new TextDecoder().decode(options.content))).toEqual(copy);
  });

  it('a layer that draws nothing is null, as today', () => {
    const transport = { rasterSync: () => null, toCanvas: canvas } as unknown as EngineRenderTransport;
    expect(engineRasterSync(transport, { locate: () => ({ composition: null, own: true }) }, { id: 'c', type: 'captions' }, 1, 0, null)).toBeNull();
  });
});
