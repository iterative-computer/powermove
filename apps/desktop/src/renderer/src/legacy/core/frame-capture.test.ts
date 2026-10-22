import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { captureFramePixels, renderOpaqueFrame, writeStraightAlphaPixels } from './frame-capture';

it('matches straight-alpha conversion for every alpha and channel byte, including row order and offset views', () => {
  const width = 256, height = 256;
  const pixels = new Uint8Array(width * height * 4 + 11).subarray(7, 7 + width * height * 4);
  const output = new Uint8ClampedArray(pixels.length + 13).subarray(9, 9 + pixels.length);
  const expected = new Uint8ClampedArray(pixels.length);
  for (let alpha = 0; alpha < 256; alpha++) for (let color = 0; color < 256; color++) {
    const source = (alpha * width + color) * 4, target = ((height - 1 - alpha) * width + color) * 4;
    pixels.set([color, 255 - color, (color * 17) & 255, alpha], source);
    for (let c = 0; c < 3; c++) expected[target + c] = alpha === 0 ? 0 : Math.min(255, (pixels[source + c]! * 255 / alpha) | 0);
    expected[target + 3] = alpha;
  }
  writeStraightAlphaPixels(pixels, width, height, output);
  expect(output).toEqual(expected);
});

beforeEach(() => {
  vi.stubGlobal('ImageData', class {
    data: Uint8ClampedArray;
    width: number;
    height: number;
    constructor(data: Uint8ClampedArray, width: number, height: number) {
      this.data = data; this.width = width; this.height = height;
    }
  });
});
afterEach(() => vi.unstubAllGlobals());

function scene(pixels: Uint8Array | null = new Uint8Array(24)) {
  return { quality: .5, proj: { shutter: .75 }, GL: { renderToPixels: vi.fn(() => pixels), gl: { isContextLost: () => false } } };
}

describe('opaque frame capture', () => {
  it('shares the GPU-owned top-down buffer without copying or modifying it', () => {
    const pixels = Uint8Array.from([17,18,19,255, 21,22,23,255, 9,10,11,255, 13,14,15,255, 1,2,3,255, 5,6,7,255]);
    const PM = scene(pixels);
    const result = renderOpaqueFrame(PM, .25, 2, 3, { mblur: false });
    expect([...result.data]).toEqual([17,18,19,255, 21,22,23,255, 9,10,11,255, 13,14,15,255, 1,2,3,255, 5,6,7,255]);
    expect(result.data.buffer).toBe(pixels.buffer);
    expect(result.width).toBe(2);
    expect(result.height).toBe(3);
    expect(PM.GL.renderToPixels).toHaveBeenCalledWith(.25, 2, 3, { mblur: false, mbSamples: 1, shutter: .75, opaque: true, topDownOpaque: true });
    expect(PM.quality).toBe(.5);
  });

  it('keeps the exact byte range if readback returns a view into a buffer', () => {
    const pixels = new Uint8Array(32).subarray(4, 28);
    pixels.fill(255);
    const image = renderOpaqueFrame(scene(pixels), 0, 2, 3);
    expect(image.data.buffer).toBe(pixels.buffer);
    expect(image.data.byteOffset).toBe(4);
    expect(image.data.byteLength).toBe(24);
  });

  it('preserves motion blur sampling and renders at full quality before restoring preview quality', () => {
    const PM = scene();
    PM.GL.renderToPixels.mockImplementation(() => { expect(PM.quality).toBe(1); return new Uint8Array(24); });
    renderOpaqueFrame(PM, 0, 2, 3);
    expect(PM.GL.renderToPixels).toHaveBeenLastCalledWith(0, 2, 3, { mblur: true, mbSamples: 16, shutter: .75, opaque: true, topDownOpaque: true });
    renderOpaqueFrame(PM, 0, 2, 3, { mbSamples: 7 });
    expect(PM.GL.renderToPixels).toHaveBeenLastCalledWith(0, 2, 3, { mblur: true, mbSamples: 7, shutter: .75, opaque: true, topDownOpaque: true });
    expect(PM.quality).toBe(.5);
  });

  it.each(['missing', 'short', 'lost', 'throw'])('rejects a %s GPU frame and restores quality', failure => {
    const PM = scene(failure === 'missing' ? null : new Uint8Array(failure === 'short' ? 16 : 24));
    if (failure === 'lost') PM.GL.gl.isContextLost = () => true;
    if (failure === 'throw') PM.GL.renderToPixels.mockImplementation(() => { throw new Error('Read failed'); });
    expect(() => renderOpaqueFrame(PM, 0, 2, 3)).toThrow();
    expect(PM.quality).toBe(.5);
  });
});


describe('shared guarded capture provider', () => {
  it('passes the same options to an optional provider without rendering twice or copying its bytes', () => {
    const PM = scene(), pixels = new Uint8Array(32).subarray(4, 28);
    const options = { transparent: true, mblur: false, mbSamples: 20, shutter: .75 };
    const capture = vi.fn(() => pixels);
    expect(captureFramePixels(PM, .25, 2, 3, options, capture)).toBe(pixels);
    expect(capture).toHaveBeenCalledExactlyOnceWith(options);
    expect(PM.GL.renderToPixels).not.toHaveBeenCalled();
  });

  it.each(['null', 'length', 'lost', 'flag', 'throw'])('rejects a provider %s failure without a hidden fallback capture', failure => {
    const PM = scene();
    if (failure === 'lost') PM.GL.gl.isContextLost = () => true;
    if (failure === 'flag') (PM.GL as any).contextLost = true;
    const capture = vi.fn(() => {
      if (failure === 'throw') throw new Error('Provider failed');
      return failure === 'null' ? null : new Uint8Array(failure === 'length' ? 16 : 24);
    });
    expect(() => captureFramePixels(PM, 0, 2, 3, {}, capture)).toThrow();
    expect(capture).toHaveBeenCalledOnce();
    expect(PM.GL.renderToPixels).not.toHaveBeenCalled();
  });
});
