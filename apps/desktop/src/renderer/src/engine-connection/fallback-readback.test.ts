import { describe, expect, it } from 'vitest';

import { FallbackReadback } from './fallback-readback';

describe('float32 fallback PBO readback', () => {
  it('returns exact float32 bit patterns without binary16 conversion', async () => {
    const expected = Float32Array.from([1 + 2 ** -20, -2.125, 0.0001, 1]);
    const bits = new Uint32Array(expected.buffer).slice();
    let storage = new Uint8Array(0);
    let requestedType = -1;
    const gl = {
      PIXEL_PACK_BUFFER: 1, STREAM_READ: 2, RGBA: 3, FLOAT: 4, HALF_FLOAT: 5, UNSIGNED_BYTE: 6,
      SYNC_GPU_COMMANDS_COMPLETE: 7, ALREADY_SIGNALED: 8, CONDITION_SATISFIED: 9,
      WAIT_FAILED: 10, NO_ERROR: 0,
      createBuffer: () => ({}), bindBuffer() {},
      bufferData: (_target: number, bytes: number) => { storage = new Uint8Array(bytes); },
      readPixels: (_x: number, _y: number, _w: number, _h: number, _format: number, type: number, offset: number) => {
        requestedType = type;
        expect(offset).toBe(0);
        storage.set(new Uint8Array(expected.buffer));
      },
      fenceSync: () => ({}), flush() {}, clientWaitSync: () => 8, deleteSync() {},
      getBufferSubData: (_target: number, _offset: number, out: Float32Array) => {
        new Uint8Array(out.buffer, out.byteOffset, out.byteLength).set(storage);
      },
      getError: () => 0, isContextLost: () => false, deleteBuffer() {},
      getParameter: () => null,
    } as unknown as WebGL2RenderingContext;
    const readback = new FallbackReadback(gl);
    const values = await readback.read(1, 1, 32);
    expect(requestedType).toBe(gl.FLOAT);
    expect(values).toBeInstanceOf(Float32Array);
    expect(new Uint32Array(values.buffer, values.byteOffset, values.length)).toEqual(bits);
    readback.dispose();
  });
});
