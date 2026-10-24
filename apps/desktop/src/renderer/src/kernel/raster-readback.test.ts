import { afterEach, describe, expect, it, vi } from 'vitest';
import { createRasterReadback } from './raster-readback';

function setup() {
  const bitmap = { close: vi.fn() };
  const capture = vi.fn(async () => bitmap);
  const workers: FakeWorker[] = [];
  class FakeWorker {
    onmessage: ((event: any) => void) | null = null;
    onerror: (() => void) | null = null;
    onmessageerror: (() => void) | null = null;
    terminate = vi.fn();
    postMessage = vi.fn();
    constructor() { workers.push(this); }
  }
  vi.stubGlobal('Worker', FakeWorker);
  vi.stubGlobal('createImageBitmap', capture);
  const source = { width: 2, height: 1 } as unknown as HTMLCanvasElement;
  return { bitmap, capture, workers, source, readback: createRasterReadback() };
}

afterEach(() => vi.unstubAllGlobals());

describe('asynchronous compositor readback', () => {
  it('separates delayed bitmap capture, worker reply and worker compute timings', async () => {
    const { capture, bitmap, workers, source, readback } = setup();
    let clock = 0, finishCapture!: (bitmap: any) => void;
    const now = vi.spyOn(performance, 'now').mockImplementation(() => clock);
    capture.mockImplementation(() => new Promise(resolve => { finishCapture = resolve; }));
    const read = async (start: number, captured: number, replied: number, computeMs: number, id: number) => {
      clock = start;
      const pending = readback.read(source);
      clock = captured; finishCapture(bitmap);
      await Promise.resolve();
      clock = replied;
      workers[0]!.onmessage!({ data: { id, pixels: null, computeMs } });
      await expect(pending).resolves.toBeNull();
    };
    try {
      await read(100, 125, 205, 50, 1);
      await read(300, 305, 325, 4, 2);
      expect(readback.stats).toEqual({ bitmapCaptures: 2, bitmapMs: 30, bitmapMsMax: 25,
        workerReplies: 2, workerMs: 100, workerMsMax: 80, workerComputeMs: 54, workerComputeMsMax: 50 });
    } finally { readback.dispose(); now.mockRestore(); }
  });

  it('captures each source immediately and matches out-of-order replies by id', async () => {
    const { capture, bitmap, workers, source, readback } = setup();
    const first = readback.read(source), second = readback.read(source);
    expect(capture).toHaveBeenCalledTimes(2);
    expect(capture).toHaveBeenCalledWith(source, { premultiplyAlpha: 'premultiply', colorSpaceConversion: 'none' });
    await Promise.resolve();
    expect(workers).toHaveLength(1);
    expect(workers[0]!.postMessage).toHaveBeenCalledWith({ id: 1, bitmap }, [bitmap]);
    const pixels = { width: 2, height: 1, format: 'rgba8Premultiplied', data: new Uint8Array(8) };
    workers[0]!.onmessage!({ data: { id: 2, pixels } });
    await expect(second).resolves.toBe(pixels);
    workers[0]!.onmessage!({ data: { id: 1, pixels: null } });
    await expect(first).resolves.toBeNull();
    readback.dispose();
    expect(workers[0]!.terminate).toHaveBeenCalledOnce();
  });

  it('closes a captured bitmap if disposed before its capture finishes', async () => {
    const { capture, bitmap, source, readback, workers } = setup();
    let finish!: (bitmap: any) => void;
    capture.mockImplementation(() => new Promise(resolve => { finish = resolve; }));
    const result = readback.read(source);
    readback.dispose();
    finish(bitmap);
    await expect(result).rejects.toThrow('disposed');
    expect(bitmap.close).toHaveBeenCalledOnce();
    expect(workers).toHaveLength(0);
    await expect(readback.read(source)).rejects.toThrow('disposed');
  });

  it('rejects pending jobs and terminates the worker on disposal', async () => {
    const { source, readback, workers } = setup();
    const pending = readback.read(source);
    await Promise.resolve();
    readback.dispose();
    await expect(pending).rejects.toThrow('disposed');
    expect(workers[0]!.terminate).toHaveBeenCalledOnce();
  });

  it('reports worker failures and creates a fresh worker on a later request', async () => {
    const { source, readback, workers } = setup();
    const first = readback.read(source), second = readback.read(source);
    await Promise.resolve();
    workers[0]!.onerror!();
    await expect(first).rejects.toThrow('worker failed');
    await expect(second).rejects.toThrow('worker failed');
    expect(workers[0]!.terminate).toHaveBeenCalledOnce();
    const retry = readback.read(source);
    await Promise.resolve();
    expect(workers).toHaveLength(2);
    workers[1]!.onmessage!({ data: { id: 3, error: 'readback failed' } });
    await expect(retry).rejects.toThrow('readback failed');
    readback.dispose();
  });

  it('closes the bitmap when worker startup or transfer fails', async () => {
    const { source, readback, workers, bitmap } = setup();
    vi.stubGlobal('Worker', class { constructor() { throw new Error('startup failed'); } });
    await expect(readback.read(source)).rejects.toThrow('startup failed');
    expect(bitmap.close).toHaveBeenCalledOnce();
    readback.dispose();
    expect(workers).toHaveLength(0);

    const next = setup();
    const result = next.readback.read(next.source);
    // Capture finishes in a microtask, so install the failing transfer in
    // the constructor before that continuation creates its worker.
    vi.stubGlobal('Worker', class { postMessage() { throw new Error('transfer failed'); } terminate() {} });
    await expect(result).rejects.toThrow('transfer failed');
    expect(next.bitmap.close).toHaveBeenCalledOnce();
    next.readback.dispose();
  });
});
