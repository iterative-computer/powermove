import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createOpaqueReadback } from './opaque-readback';

function setup() {
  const originalBinding = { original: true }, buffer = { capture: true };
  let binding: unknown = originalBinding, size = 0, lost = false, current = true;
  let status = 21, captureError: 'before' | 'after' | undefined;
  const gl = {
    PIXEL_PACK_BUFFER: 1, PIXEL_PACK_BUFFER_BINDING: 2, STREAM_READ: 3, BUFFER_SIZE: 4,
    RGBA: 5, UNSIGNED_BYTE: 6, SYNC_GPU_COMMANDS_COMPLETE: 7,
    ALREADY_SIGNALED: 21, CONDITION_SATISFIED: 22, TIMEOUT_EXPIRED: 23, WAIT_FAILED: 24,
    isContextLost: vi.fn(() => lost), createBuffer: vi.fn(() => buffer), deleteBuffer: vi.fn(),
    getParameter: vi.fn(() => binding), bindBuffer: vi.fn((_target, value) => { binding = value; }),
    bufferData: vi.fn((_target, bytes) => { size = bytes; }), getBufferParameter: vi.fn(() => size),
    readPixels: vi.fn(), fenceSync: vi.fn((): object | null => ({})), deleteSync: vi.fn(), flush: vi.fn(),
    clientWaitSync: vi.fn(() => status),
    getBufferSubData: vi.fn((_target, _offset, target: Uint8Array) => {
      expect(binding).toBe(buffer);
      target.set([0, 17, 129, 255, 255, 1, 2, 255]);
    }),
  };
  const capture = vi.fn((_time: number, _options: Record<string, unknown>, read: () => void) => {
    if (captureError === 'before') throw new Error('Render failed');
    read();
    if (captureError === 'after') throw new Error('Render failed');
  });
  const create = () => createOpaqueReadback(gl as any, 2, 1, capture, () => current);
  return { gl, capture, create, originalBinding, buffer, binding: () => binding,
    lose: () => { lost = true; }, replace: () => { current = false; },
    status: (value: number) => { status = value; },
    failCapture: (value?: typeof captureError) => { captureError = value; } };
}

beforeEach(() => { vi.useFakeTimers(); });
afterEach(() => { vi.useRealTimers(); vi.restoreAllMocks(); });

describe('opaque GPU readback', () => {
  it('rejects unsupported contexts and invalid dimensions before allocating or capturing', () => {
    const h = setup();
    expect(createOpaqueReadback(null, 2, 1, h.capture, () => true)).toBeNull();
    expect(createOpaqueReadback(h.gl as any, 0, 1, h.capture, () => true)).toBeNull();
    expect(createOpaqueReadback({ ...h.gl, fenceSync: undefined } as any, 2, 1, h.capture, () => true)).toBeNull();
    h.lose(); expect(h.create()).toBeNull();
    expect(h.gl.createBuffer).not.toHaveBeenCalled(); expect(h.capture).not.toHaveBeenCalled();
  });

  it.each(['null', 'size', 'throw'] as const)('falls back on allocation failure and restores the previous pack buffer: %s', failure => {
    const h = setup();
    if (failure === 'null') h.gl.createBuffer.mockReturnValueOnce(null as any);
    if (failure === 'size') h.gl.getBufferParameter.mockReturnValueOnce(0);
    if (failure === 'throw') h.gl.bufferData.mockImplementationOnce(() => { throw new Error('Allocation failed'); });
    expect(h.create()).toBeNull();
    expect(h.binding()).toBe(h.originalBinding); expect(h.capture).not.toHaveBeenCalled();
    expect(h.gl.deleteBuffer).toHaveBeenCalledTimes(failure === 'null' ? 0 : 1);
  });

  it('queues a capture without CPU pixels and waits for an event-loop task before copying into caller storage', async () => {
    const h = setup(), session = h.create()!, target = new Uint8Array(8);
    const options = { mblur: false, shutter: .5 };
    const ticket = session.begin(.25, options);
    expect(h.capture).toHaveBeenCalledWith(.25, options, expect.any(Function));
    expect(h.gl.readPixels).toHaveBeenCalledWith(0, 0, 2, 1, h.gl.RGBA, h.gl.UNSIGNED_BYTE, 0);
    expect(h.binding()).toBe(h.originalBinding);
    const reading = ticket.readInto(target, () => false);
    expect(h.gl.clientWaitSync).not.toHaveBeenCalled(); expect(h.gl.getBufferSubData).not.toHaveBeenCalled();
    expect([...target]).toEqual(Array(8).fill(0));
    await vi.runAllTimersAsync();
    expect(await reading).toBe(true);
    expect([...target]).toEqual([0, 17, 129, 255, 255, 1, 2, 255]);
    expect(h.binding()).toBe(h.originalBinding); expect(h.gl.deleteSync).toHaveBeenCalledOnce();
    // A second capture reuses the GPU buffer, and dispose is idempotent.
    session.begin(.5).dispose(); session.dispose(); session.dispose();
    expect(h.gl.createBuffer).toHaveBeenCalledOnce(); expect(h.gl.bufferData).toHaveBeenCalledOnce();
    expect(h.gl.deleteBuffer).toHaveBeenCalledExactlyOnceWith(h.buffer); expect(vi.getTimerCount()).toBe(0);
  });

  it('reserves one ticket before rendering, including reentrant callers', () => {
    const h = setup(), session = h.create()!;
    h.capture.mockImplementationOnce((_time, _options, read) => {
      expect(() => session.begin(1)).toThrow('already pending'); read();
    });
    const ticket = session.begin(0);
    expect(() => session.begin(1)).toThrow('already pending');
    ticket.dispose(); session.begin(1).dispose(); session.dispose();
    expect(h.capture).toHaveBeenCalledTimes(2);
  });

  it.each(['before', 'after'] as const)('releases failed renders and permits a later capture: %s readback submission', failure => {
    const h = setup(), session = h.create()!; h.failCapture(failure);
    expect(() => session.begin(0)).toThrow('Render failed');
    expect(h.binding()).toBe(h.originalBinding); expect(h.gl.deleteSync).toHaveBeenCalledTimes(failure === 'after' ? 1 : 0);
    h.failCapture(); session.begin(1).dispose(); session.dispose();
  });

  it('cleans up a missing fence without keeping the session busy', () => {
    const h = setup(), session = h.create()!; h.gl.fenceSync.mockReturnValueOnce(null);
    expect(() => session.begin(0)).toThrow('GPU');
    expect(h.binding()).toBe(h.originalBinding); session.begin(1).dispose(); session.dispose();
  });

  it.each(['cancel', 'ticket', 'session'] as const)('wakes a pending read and never copies pixels on %s', async reason => {
    const h = setup(), session = h.create()!, ticket = session.begin(0);
    h.status(h.gl.TIMEOUT_EXPIRED);
    let cancel = false;
    const reading = ticket.readInto(new Uint8Array(8), () => cancel);
    if (reason === 'cancel') cancel = true;
    if (reason === 'ticket') ticket.dispose();
    if (reason === 'session') session.dispose();
    await vi.runAllTimersAsync(); expect(await reading).toBe(false);
    expect(h.gl.getBufferSubData).not.toHaveBeenCalled(); expect(h.gl.deleteSync).toHaveBeenCalledOnce();
    expect(h.binding()).toBe(h.originalBinding); expect(vi.getTimerCount()).toBe(0);
    if (reason !== 'session') session.begin(1).dispose(); session.dispose();
  });

  it.each(['lost', 'replaced', 'wait', 'wait-throw', 'copy', 'lost-during-copy', 'timeout'] as const)('rejects GPU/readback failure and cleans state: %s', async reason => {
    const h = setup(), session = h.create()!, ticket = session.begin(0);
    const reading = ticket.readInto(new Uint8Array(8), () => false);
    const rejected = expect(reading).rejects.toThrow(/GPU/);
    if (reason === 'lost') h.lose();
    if (reason === 'replaced') h.replace();
    if (reason === 'wait') h.status(h.gl.WAIT_FAILED);
    if (reason === 'wait-throw') h.gl.clientWaitSync.mockImplementationOnce(() => { throw new Error('GPU wait failed'); });
    if (reason === 'copy') h.gl.getBufferSubData.mockImplementationOnce(() => { throw new Error('GPU copy failed'); });
    if (reason === 'lost-during-copy') h.gl.getBufferSubData.mockImplementationOnce(() => { h.lose(); });
    if (reason === 'timeout') vi.spyOn(performance, 'now').mockReturnValue(performance.now() + 10001);
    await vi.runAllTimersAsync(); await rejected;
    expect(h.binding()).toBe(h.originalBinding); expect(h.gl.deleteSync).toHaveBeenCalledOnce();
    expect(vi.getTimerCount()).toBe(0); session.dispose();
  });

  it('cleans up an invalid destination and a throwing cancellation check', async () => {
    const h = setup(), session = h.create()!;
    await expect(session.begin(0).readInto(new Uint8Array(4), () => false)).rejects.toThrow('wrong size');
    await expect(session.begin(0).readInto(new Uint8ClampedArray(8) as any, () => false)).rejects.toThrow('type');
    await expect(session.begin(1).readInto(new Uint8Array(8), () => { throw new Error('Cancel failed'); })).rejects.toThrow('Cancel failed');
    session.begin(2).dispose(); session.dispose(); expect(h.gl.deleteSync).toHaveBeenCalledTimes(4);
  });

  it('rejects a cancellation callback that throws from an asynchronous poll without leaving timers', async () => {
    const h = setup(), session = h.create()!, ticket = session.begin(0);
    const cancelled = vi.fn((): boolean => { throw new Error('Poll cancellation failed'); }).mockReturnValueOnce(false);
    const reading = ticket.readInto(new Uint8Array(8), cancelled);
    const rejected = expect(reading).rejects.toThrow('Poll cancellation failed');
    await vi.runAllTimersAsync(); await rejected;
    expect(h.gl.getBufferSubData).not.toHaveBeenCalled(); expect(vi.getTimerCount()).toBe(0);
    session.begin(1).dispose(); session.dispose();
  });

  it('does not queue a fence after reentrant disposal during capture', () => {
    const h = setup(), session = h.create()!;
    h.capture.mockImplementationOnce((_time, _options, read) => { session.dispose(); read(); });
    expect(() => session.begin(0)).toThrow('GPU');
    expect(h.gl.fenceSync).not.toHaveBeenCalled(); expect(h.gl.deleteBuffer).toHaveBeenCalledOnce();
  });
});
