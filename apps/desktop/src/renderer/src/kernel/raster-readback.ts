import type { compositorPixels } from './raster-pixels';

type Pixels = ReturnType<typeof compositorPixels>;
type Source = Parameters<typeof compositorPixels>[0];

export interface RasterReadbackStats {
  bitmapCaptures: number;
  bitmapMs: number;
  bitmapMsMax: number;
  workerReplies: number;
  /** Post-to-reply wall time, including worker startup and queueing. */
  workerMs: number;
  workerMsMax: number;
  /** Time inside the worker's pixel conversion, excluding its message queue. */
  workerComputeMs: number;
  workerComputeMsMax: number;
}

/** Playback readback never waits for a GPU fence on the renderer thread. */
export function createRasterReadback(stats: RasterReadbackStats = {
  bitmapCaptures: 0, bitmapMs: 0, bitmapMsMax: 0,
  workerReplies: 0, workerMs: 0, workerMsMax: 0, workerComputeMs: 0, workerComputeMsMax: 0
}): { readonly stats: RasterReadbackStats; read(source: Source): Promise<Pixels>; dispose(): void } {
  let worker: Worker | null = null;
  let serial = 0, disposed = false;
  const pending = new Map<number, { resolve(value: Pixels): void; reject(error: Error): void; started: number }>();
  const fail = (message: string) => {
    for (const job of pending.values()) job.reject(new Error(message));
    pending.clear();
    worker?.terminate(); worker = null;
  };
  return {
    stats,
    async read(source) {
      if (disposed) throw new Error('Raster readback was disposed.');
      if (typeof Worker === 'undefined' || typeof createImageBitmap === 'undefined') throw new Error('This renderer cannot prepare raster pixels asynchronously.');
      // ImageBitmap uploads ignore WebGL UNPACK_PREMULTIPLY_ALPHA_WEBGL;
      // request premultiplication explicitly, and preserve the source colors.
      const beganCapture = performance.now();
      let bitmap: ImageBitmap;
      try { bitmap = await createImageBitmap(source, { premultiplyAlpha: 'premultiply', colorSpaceConversion: 'none' }); }
      finally {
        const ms = performance.now() - beganCapture;
        stats.bitmapCaptures++; stats.bitmapMs += ms; stats.bitmapMsMax = Math.max(stats.bitmapMsMax, ms);
      }
      if (disposed) { bitmap.close(); throw new Error('Raster readback was disposed.'); }
      if (!worker) {
        try { worker = new Worker(new URL('./raster-readback-worker.ts', import.meta.url), { type: 'module' }); }
        catch (error) { bitmap.close(); throw error; }
        worker.onmessage = (event: MessageEvent<{ id: number; pixels?: Pixels; error?: string; computeMs?: number }>) => {
          const job = pending.get(event.data.id);
          if (!job) return;
          pending.delete(event.data.id);
          const ms = performance.now() - job.started;
          stats.workerReplies++; stats.workerMs += ms; stats.workerMsMax = Math.max(stats.workerMsMax, ms);
          const computeMs = event.data.computeMs;
          if (typeof computeMs === 'number' && Number.isFinite(computeMs) && computeMs >= 0) {
            stats.workerComputeMs += computeMs; stats.workerComputeMsMax = Math.max(stats.workerComputeMsMax, computeMs);
          }
          if (event.data.error) job.reject(new Error(event.data.error)); else job.resolve(event.data.pixels ?? null);
        };
        worker.onerror = () => fail('Asynchronous raster readback worker failed.');
        worker.onmessageerror = () => fail('Asynchronous raster readback returned an invalid message.');
      }
      const id = ++serial;
      return new Promise<Pixels>((resolve, reject) => {
        pending.set(id, { resolve, reject, started: performance.now() });
        try { worker!.postMessage({ id, bitmap }, [bitmap]); }
        catch (error) { pending.delete(id); bitmap.close(); reject(error); }
      });
    },
    dispose() { disposed = true; fail('Raster readback was disposed.'); }
  };
}
