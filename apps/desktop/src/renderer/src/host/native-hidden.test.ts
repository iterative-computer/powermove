import { beforeEach, describe, expect, it, vi } from 'vitest';

const render = vi.hoisted(() => vi.fn());
vi.mock('../engine-connection/shader-fallback', () => ({
  ShaderFallbackRunner: class { render = render; }
}));

import { runShader } from './native-hidden';

describe('direct CLI ANGLE transport', () => {
  beforeEach(() => render.mockReset());

  it('carries depth-32 input and output as exact binary32 bytes', async () => {
    const bits = new Uint32Array([0x3f80_0001, 0x3f00_0001, 0x0000_0001, 0xbf80_0001]);
    render.mockResolvedValueOnce({ rgba16f: new Uint16Array(0), rgba32f: new Float32Array(bits.buffer) });
    const input = Buffer.from(bits.buffer).toString('base64');
    const result = await runShader({ colorDepth: 32 }, [input]);
    expect(result.ok).toBe(true);
    const textures = render.mock.calls[0]![1] as Float32Array[];
    expect(textures[0]).toBeInstanceOf(Float32Array);
    expect(new Uint32Array(textures[0]!.buffer)).toEqual(bits);
    if (!result.ok) throw new Error(result.message);
    const output = Buffer.from(result.pixels, 'base64');
    expect(new Uint32Array(Uint8Array.from(output).buffer)).toEqual(bits);
  });

  it('keeps depth-8 codes in the u16 wire slots', async () => {
    const codes = new Uint16Array([0, 127, 255, 255]);
    render.mockResolvedValueOnce({ rgba16f: codes });
    const result = await runShader({ colorDepth: 8 }, [Buffer.from(codes.buffer).toString('base64')]);
    expect(result.ok).toBe(true);
    const textures = render.mock.calls[0]![1] as Uint16Array[];
    expect(textures[0]).toBeInstanceOf(Uint16Array);
    expect(textures[0]).toEqual(codes);
    if (!result.ok) throw new Error(result.message);
    expect(Buffer.from(result.pixels, 'base64')).toEqual(Buffer.from(codes.buffer));
  });
});
