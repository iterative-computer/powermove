import type { compositorPixels } from './raster-pixels';

type Pixels = ReturnType<typeof compositorPixels>;
type Source = Parameters<typeof compositorPixels>[0];

/** Playback readback never waits for a GPU fence on the renderer thread. */
export function createRasterReadback(): { read(source: Source): Promise<Pixels>; dispose(): void } {
  let worker: Worker | null = null;
  let serial = 0, disposed = false;
  const pending = new Map<number, { resolve(value: Pixels): void; reject(error: Error): void }>();
  const fail = (message: string) => {
    for (const job of pending.values()) job.reject(new Error(message));
    pending.clear();
    worker?.terminate(); worker = null;
  };
  return {
    async read(source) {
      if (disposed) throw new Error('Raster readback was disposed.');
      if (typeof Worker === 'undefined' || typeof createImageBitmap === 'undefined') throw new Error('This renderer cannot prepare raster pixels asynchronously.');
      // ImageBitmap uploads ignore WebGL UNPACK_PREMULTIPLY_ALPHA_WEBGL;
      // request premultiplication explicitly, and preserve the source colors.
      const bitmap = await createImageBitmap(source, { premultiplyAlpha: 'premultiply', colorSpaceConversion: 'none' });
      if (disposed) { bitmap.close(); throw new Error('Raster readback was disposed.'); }
      if (!worker) {
        try { worker = new Worker(new URL('./raster-readback-worker.ts', import.meta.url), { type: 'module' }); }
        catch (error) { bitmap.close(); throw error; }
        worker.onmessage = (event: MessageEvent<{ id: number; pixels?: Pixels; error?: string }>) => {
          const job = pending.get(event.data.id);
          if (!job) return;
          pending.delete(event.data.id);
          if (event.data.error) job.reject(new Error(event.data.error)); else job.resolve(event.data.pixels ?? null);
        };
        worker.onerror = () => fail('Asynchronous raster readback worker failed.');
        worker.onmessageerror = () => fail('Asynchronous raster readback returned an invalid message.');
      }
      const id = ++serial;
      return new Promise<Pixels>((resolve, reject) => {
        pending.set(id, { resolve, reject });
        try { worker!.postMessage({ id, bitmap }, [bitmap]); }
        catch (error) { pending.delete(id); bitmap.close(); reject(error); }
      });
    },
    dispose() { disposed = true; fail('Raster readback was disposed.'); }
  };
}
