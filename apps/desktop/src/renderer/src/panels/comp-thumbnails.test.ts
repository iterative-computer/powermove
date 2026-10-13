// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createCompThumbnails, posterTime, thumbnailSize, type CompThumbnails } from './comp-thumbnails.svelte';

let thumbnails: CompThumbnails[] = [];
let createdCanvases = 0;
let canvases: HTMLCanvasElement[] = [];
const putImageData = vi.fn();

beforeEach(() => {
  vi.useFakeTimers(); createdCanvases = 0; canvases = [];
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue({ putImageData } as any);
  vi.spyOn(HTMLCanvasElement.prototype, 'toDataURL').mockImplementation(function (this: HTMLCanvasElement) {
    return `data:image/jpeg;${this.width}x${this.height};${putImageData.mock.calls.length}`;
  });
  const nativeCreate = document.createElement.bind(document);
  vi.spyOn(document, 'createElement').mockImplementation((tag: string, options?: ElementCreationOptions) => {
    const element = nativeCreate(tag, options);
    if (tag === 'canvas') { createdCanvases++; canvases.push(element as HTMLCanvasElement); }
    return element;
  });
  vi.stubGlobal('ImageData', class { constructor(readonly data: Uint8ClampedArray, readonly width: number, readonly height: number) {} });
});
afterEach(() => {
  for (const cache of thumbnails) cache.dispose();
  thumbnails = []; vi.useRealTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals(); putImageData.mockClear();
});

function fixture(count = 3) {
  const comps: any[] = Array.from({ length: count }, (_, index) => ({ id: `comp-${index}`, w: 320, h: 180, dur: 8, work: [2, 6], layers: [] }));
  const viewer = { isNavigating: vi.fn(() => false) };
  const PM: any = {
    proj: { ...comps[0], compId: comps[0]?.id }, quality: .5, playing: false,
    interactionActive: vi.fn(() => false), Kernel: { services: new Map([['viewer', viewer]]) },
    Comps: { list: vi.fn(() => comps), get: vi.fn((id: string) => comps.find(comp => comp.id === id)) },
    GL: { renderToPixels: vi.fn((_time: number, w: number, h: number) => new Uint8Array(w * h * 4)), gl: { isContextLost: () => false } },
  };
  const cache = createCompThumbnails(PM); thumbnails.push(cache);
  return { PM, cache, comps, viewer };
}

describe('composition thumbnails', () => {
  it('reuses one detached canvas across every composition while keeping independent encoded images', () => {
    const { PM, cache } = fixture(200);
    cache.refresh('first'); vi.runAllTimers();
    expect(PM.GL.renderToPixels).toHaveBeenCalledTimes(200);
    expect(Object.keys(cache.urls)).toHaveLength(200);
    expect(new Set(Object.values(cache.urls)).size).toBe(200);
    expect(createdCanvases).toBe(1);
    expect(document.querySelector('canvas')).toBeNull();
    expect(PM.quality).toBe(.5);
    const call = PM.GL.renderToPixels.mock.calls[0];
    expect(call.slice(0, 3)).toEqual([4, 320, 180]);
    expect(call[3]).toMatchObject({ mblur: false, mbSamples: 1, opaque: true, topDownOpaque: true, comp: PM.proj });
  });

  it('preserves poster times, aspect ratios and pixel inputs when the shared canvas changes size', () => {
    const { PM, comps, cache } = fixture();
    comps[1].w = 120; comps[1].h = 240; comps[1].work = [1, 5];
    comps[2].w = 1280; comps[2].h = 720; comps[2].work = [7, 30];
    PM.GL.renderToPixels.mockImplementation((_time: number, w: number, h: number) => {
      const pixels = new Uint8Array(w * h * 4); pixels.fill(w % 256); return pixels;
    });
    cache.refresh('first'); vi.runAllTimers();
    expect(PM.GL.renderToPixels.mock.calls.map((call: any[]) => call.slice(0, 3))).toEqual([[4, 320, 180], [3, 120, 240], [7.5, 320, 180]]);
    expect(putImageData.mock.calls.map(call => [call[0].width, call[0].height, call[0].data[0]])).toEqual([[320, 180, 64], [120, 240, 120], [320, 180, 64]]);
    expect(cache.urls['comp-1']).toContain('120x240');
    expect(cache.urls['comp-2']).toContain('320x180');
  });

  it('keeps existing thumbnails through edits, updates new comps and removes deleted comps', () => {
    const { PM, comps, cache } = fixture();
    cache.refresh('first'); vi.runAllTimers();
    const old = { ...cache.urls };
    comps.splice(1, 1); comps.push({ ...comps[0], id: 'added' });
    cache.refresh('second'); vi.advanceTimersByTime(399);
    expect({ ...cache.urls }).toEqual(old);
    vi.runAllTimers();
    expect(Object.keys(cache.urls)).toEqual(['comp-0', 'comp-2', 'added']);
    expect(cache.urls['comp-0']).not.toBe(old['comp-0']);
    PM.GL.renderToPixels.mockClear();
    cache.refresh('second'); vi.runAllTimers();
    expect(PM.GL.renderToPixels).not.toHaveBeenCalled();
    expect(createdCanvases).toBe(1);
  });

  it.each(['interaction', 'navigation', 'playing', 'capture', 'export', 'preparing', 'preview'])('defers GPU readbacks during %s and resumes afterwards', blocker => {
    const { PM, cache, viewer } = fixture();
    if (blocker === 'interaction') PM.interactionActive.mockReturnValue(true);
    if (blocker === 'navigation') viewer.isNavigating.mockReturnValue(true);
    if (blocker === 'playing') PM.playing = true;
    if (blocker === 'capture') PM.agentFrameCapture = true;
    if (blocker === 'export') PM.Export = { busy: true };
    if (blocker === 'preparing') PM.Preview = { preparing: true };
    if (blocker === 'preview') PM.Preview = { active: true };
    cache.refresh('first'); vi.advanceTimersByTime(900);
    expect(PM.GL.renderToPixels).not.toHaveBeenCalled();
    expect(createdCanvases).toBe(0);
    PM.interactionActive.mockReturnValue(false); viewer.isNavigating.mockReturnValue(false);
    PM.playing = false; PM.agentFrameCapture = false; PM.Export = {}; PM.Preview = {};
    vi.runAllTimers();
    expect(PM.GL.renderToPixels).toHaveBeenCalledTimes(3);
    expect(Object.keys(cache.urls)).toHaveLength(3);
  });

  it('restores render quality after failed reads, avoids a retry loop and preserves the last good image', () => {
    const { PM, cache } = fixture(1);
    vi.spyOn(console, 'warn').mockImplementation(() => {});
    cache.refresh('first'); vi.runAllTimers();
    const old = cache.urls['comp-0'];
    PM.GL.renderToPixels.mockImplementationOnce(() => { throw new Error('GPU unavailable'); });
    cache.refresh('second'); vi.runAllTimers();
    expect(PM.quality).toBe(.5); expect(cache.urls['comp-0']).toBe(old);
    expect(PM.GL.renderToPixels).toHaveBeenCalledTimes(2);
    cache.refresh('second'); vi.runAllTimers();
    expect(PM.GL.renderToPixels).toHaveBeenCalledTimes(2);
    cache.refresh('third'); vi.runAllTimers();
    expect(cache.urls['comp-0']).not.toBe(old);
    expect(createdCanvases).toBe(1);
  });

  it('stops scheduled work and releases its canvas on disposal', () => {
    const { PM, cache } = fixture();
    cache.refresh('first'); cache.dispose(); cache.refresh('second'); vi.runAllTimers();
    expect(PM.GL.renderToPixels).not.toHaveBeenCalled();
    expect(createdCanvases).toBe(0); expect(vi.getTimerCount()).toBe(0);
  });

  it('releases the allocated backing store and cancels a pending replacement on disposal', () => {
    const { PM, cache } = fixture();
    cache.refresh('first'); vi.runAllTimers();
    expect(canvases[0]?.width).toBe(320); expect(canvases[0]?.height).toBe(180);
    cache.refresh('second'); cache.dispose(); vi.runAllTimers();
    expect(canvases[0]?.width).toBe(0); expect(canvases[0]?.height).toBe(0);
    expect(PM.GL.renderToPixels).toHaveBeenCalledTimes(3);
    expect(createdCanvases).toBe(1); expect(vi.getTimerCount()).toBe(0);
  });

  it('clamps poster times and minimum dimensions for malformed composition metadata', () => {
    expect(posterTime({ dur: 10, work: [-5, 30] })).toBe(5);
    expect(posterTime({ dur: 10, work: [8, 2] })).toBe(8);
    expect(posterTime({ dur: 10, work: [NaN, Infinity] })).toBe(5);
    expect(thumbnailSize(0, -1)).toEqual({ width: 2, height: 2 });
  });
});
