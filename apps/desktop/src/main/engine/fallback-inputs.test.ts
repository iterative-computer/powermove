import { describe, expect, it } from 'vitest';

import { FallbackInputs } from './fallback-inputs';

const KEY = 'a'.repeat(64);
const band = (texture: number, row: number, rows: number, value: number, key = KEY, width = 2, height = 4) => ({
  type: 'shaderFallback', fallback: 'input', key, texture, width, height, row, rows, rgba16f: new Uint8Array(width * rows * 8).fill(value)
});

describe('FallbackInputs', () => {
  it('assembles every band of every texture and hands them over once', () => {
    const inputs = new FallbackInputs();
    expect(inputs.receive(band(0, 0, 2, 1))).toBe(true);
    expect(inputs.receive(band(1, 0, 4, 3))).toBe(true);
    expect(inputs.receive(band(0, 2, 2, 2))).toBe(true);
    const textures = inputs.take(KEY, 2, 2, 4)!;
    expect(textures.map((t) => t.byteLength)).toEqual([64, 64]);
    expect([textures[0]![0], textures[0]![63], textures[1]![0]]).toEqual([1, 2, 3]);
    expect(inputs.take(KEY, 2, 2, 4)).toBeNull();
    expect(inputs.held).toBe(0);
  });

  it('answers null for a texture with a missing row or another size, and forgets it', () => {
    const inputs = new FallbackInputs();
    inputs.receive(band(0, 0, 3, 1));
    expect(inputs.take(KEY, 1, 2, 4)).toBeNull();
    inputs.receive(band(0, 0, 4, 1));
    expect(inputs.take(KEY, 1, 4, 4)).toBeNull();
    expect(inputs.held).toBe(0);
  });

  it('refuses damaged bands', () => {
    const inputs = new FallbackInputs();
    expect(inputs.receive({ ...band(0, 0, 2, 1), rgba16f: new Uint8Array(3) })).toBe(false);
    expect(inputs.receive(band(0, 3, 2, 1))).toBe(false);
    expect(inputs.receive({ ...band(0, 0, 1, 1), key: '' })).toBe(false);
    expect(inputs.receive({ ...band(0, 0, 1, 1), texture: 99 })).toBe(false);
    expect(inputs.held).toBe(0);
  });

  it('keeps at most its limits, dropping the oldest requests first', () => {
    const inputs = new FallbackInputs({ keys: 2, bytes: 1024 });
    for (const k of ['b', 'c', 'd']) inputs.receive(band(0, 0, 4, 1, k.repeat(64)));
    expect(inputs.take('b'.repeat(64), 1, 2, 4)).toBeNull();
    expect(inputs.take('d'.repeat(64), 1, 2, 4)).not.toBeNull();
    // One request alone over the byte limit is not kept.
    expect(inputs.receive(band(0, 0, 1, 1, 'e'.repeat(64), 64, 4))).toBe(true);
    expect(inputs.take('e'.repeat(64), 1, 64, 4)).toBeNull();
    expect(inputs.held).toBeLessThanOrEqual(1024);
  });
});
