import { compositorPixels } from './raster-pixels';

type ReadbackRequest = { id: number; bitmap: ImageBitmap };
const scope = self as unknown as {
  onmessage: ((event: MessageEvent<ReadbackRequest>) => void) | null;
  postMessage(message: unknown, transfer?: Transferable[]): void;
};
scope.onmessage = (event: MessageEvent<{ id: number; bitmap: ImageBitmap }>) => {
  const { id, bitmap } = event.data;
  const began = performance.now();
  try {
    const pixels = compositorPixels(bitmap);
    scope.postMessage({ id, pixels, computeMs: performance.now() - began }, pixels ? [pixels.data.buffer as ArrayBuffer] : []);
  } catch (error) {
    scope.postMessage({ id, error: `Asynchronous raster readback failed: ${error instanceof Error ? error.message : String(error)}`, computeMs: performance.now() - began });
  } finally { bitmap.close(); }
};
