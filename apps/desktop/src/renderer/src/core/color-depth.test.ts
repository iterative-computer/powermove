import { describe, expect, it } from 'vitest';
import { availableColorDepth, colorBufferBytes, normalizeColorDepth, supportedColorDepths } from './color-depth';

describe('working color depth', () => {
  it('keeps legacy projects at 16-bit and normalizes malformed saved values', () => {
    for (const value of [undefined, null, '32', 12, NaN]) expect(normalizeColorDepth(value)).toBe(16);
  });

  it('requires rendering, filtering and blending support for 32-bit', () => {
    const extensions = ['EXT_color_buffer_float', 'OES_texture_float_linear', 'EXT_float_blend'];
    for (const missing of [null, ...extensions]) {
      const gl = { getExtension: (name: string) => extensions.includes(name) && name !== missing ? {} : null };
      expect(supportedColorDepths(gl)).toEqual(missing === null ? [8, 16, 32] : missing === extensions[0] ? [8] : [8, 16]);
    }
    expect(supportedColorDepths({ getExtension: () => null })).toEqual([8]);
  });

  it('uses a supported fallback for portable projects without changing the saved preference', () => {
    expect(availableColorDepth(32, [8, 16])).toBe(16);
    expect(availableColorDepth(16, [8])).toBe(8);
    expect(availableColorDepth(8, [8, 16, 32])).toBe(8);
  });

  it('counts all four channels and the optional depth attachment', () => {
    expect(colorBufferBytes(100, 100, 8)).toBe(40_000);
    expect(colorBufferBytes(100, 100, 16)).toBe(80_000);
    expect(colorBufferBytes(100, 100, 32)).toBe(160_000);
    expect(colorBufferBytes(100, 100, 32, true)).toBe(180_000);
  });
});
