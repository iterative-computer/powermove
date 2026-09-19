import { describe, expect, it } from 'vitest';

import { decodeFrame, encodeFrame, isClientMessage } from './wire';

describe('wire codec', () => {
  it('round-trips JSON with nested byte arrays', () => {
    const image = new Uint8Array([1, 2, 3, 250, 251]);
    const message = {
      t: 'invoke',
      id: 7,
      ch: 'codex:run',
      args: [{ prompt: 'hi', images: [image, new Uint8Array(0)], nested: { data: image.subarray(1, 3) }, keep: null }]
    };
    const decoded = decodeFrame(encodeFrame(message)) as typeof message;
    expect(decoded.t).toBe('invoke');
    expect(decoded.id).toBe(7);
    const arg = decoded.args[0]!;
    expect(arg.prompt).toBe('hi');
    expect(arg.images[0]).toBeInstanceOf(Uint8Array);
    expect([...arg.images[0]!]).toEqual([1, 2, 3, 250, 251]);
    expect(arg.images[1]!.byteLength).toBe(0);
    expect([...(arg.nested.data as Uint8Array)]).toEqual([2, 3]);
    expect(arg.keep).toBeNull();
  });

  it('copies blobs out of the frame buffer', () => {
    const frame = encodeFrame({ t: 'send', ch: 'x', args: [new Uint8Array([9, 9])] });
    const decoded = decodeFrame(frame) as { args: [Uint8Array] };
    frame.fill(0);
    expect([...decoded.args[0]]).toEqual([9, 9]);
  });

  it('drops undefined properties like JSON does', () => {
    const decoded = decodeFrame(encodeFrame({ a: undefined, b: 1 })) as Record<string, unknown>;
    expect(decoded).toEqual({ b: 1 });
  });

  it('rejects truncated frames', () => {
    const frame = encodeFrame({ t: 'send', ch: 'x', args: [new Uint8Array(10)] });
    expect(() => decodeFrame(frame.subarray(0, frame.byteLength - 3))).toThrow(/truncated/);
    expect(() => decodeFrame(new Uint8Array(2))).toThrow(/truncated/);
  });

  it('validates client message shapes', () => {
    expect(isClientMessage({ t: 'invoke', id: 1, ch: 'a', args: [] })).toBe(true);
    expect(isClientMessage({ t: 'sync', id: 1, ch: 'a', args: [] })).toBe(true);
    expect(isClientMessage({ t: 'send', ch: 'a', args: [] })).toBe(true);
    expect(isClientMessage({ t: 'answer', id: 3, value: 0 })).toBe(true);
    expect(isClientMessage({ t: 'invoke', ch: 'a', args: [] })).toBe(false);
    expect(isClientMessage({ t: 'event', ch: 'a', args: [] })).toBe(false);
    expect(isClientMessage(null)).toBe(false);
  });
});
