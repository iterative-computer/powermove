import { finishBlas } from './blas';

// Builds one mesh's BVH off the main thread (see blas.ts).
self.onmessage = (e: MessageEvent<{ corners: Float32Array; shading: Float32Array }>) => {
  const b = finishBlas(e.data.corners, e.data.shading);
  (self as any).postMessage(b, [b.positions.buffer, b.tris.buffer, b.nodes]);
};
