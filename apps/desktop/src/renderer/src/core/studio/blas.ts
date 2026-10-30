import { buildBvh } from './bvh';

/*
 * One mesh's bottom-level BVH: its triangles' corners and shading data in BVH
 * order. Building it is the slow part of preparing a large model, so it runs
 * in a pool of workers (blas.worker.ts) while the editor keeps drawing with
 * the classic renderer; exports build in place.
 */

export type BlasData = { positions: Float32Array; tris: Float32Array; nodes: ArrayBuffer; triangles: number };

/** `corners`: 9 floats per triangle; `shading`: 24 floats per triangle. */
export function finishBlas(corners: Float32Array, shading: Float32Array): BlasData {
  const triangles = corners.length / 9;
  if (!triangles) return { positions: new Float32Array(0), tris: new Float32Array(0), nodes: new ArrayBuffer(64), triangles: 0 };
  const { nodes, order } = buildBvh(corners);
  const positions = new Float32Array(triangles * 12);
  const tris = new Float32Array(triangles * 24);
  for (let i = 0; i < triangles; i++) {
    const t = order[i]!;
    for (let c = 0; c < 3; c++) {
      positions[i * 12 + c * 4] = corners[t * 9 + c * 3]!;
      positions[i * 12 + c * 4 + 1] = corners[t * 9 + c * 3 + 1]!;
      positions[i * 12 + c * 4 + 2] = corners[t * 9 + c * 3 + 2]!;
    }
    tris.set(shading.subarray(t * 24, t * 24 + 24), i * 24);
  }
  return { positions, tris, nodes, triangles };
}

/** Meshes at least this large build in a worker; smaller ones build at once. */
export const WORKER_TRIANGLES = 20_000;

type Job = { corners: Float32Array; shading: Float32Array; resolve: (b: BlasData) => void; reject: (e: unknown) => void };
const queue: Job[] = [];
const idle: Worker[] = [];
let workers = 0;
const maxWorkers = Math.max(1, Math.min(4, ((globalThis as any).navigator?.hardwareConcurrency ?? 4) - 1));

function pump(): void {
  while (queue.length) {
    let worker = idle.pop();
    if (!worker && workers < maxWorkers) {
      try {
        worker = new Worker(new URL('./blas.worker.ts', import.meta.url), { type: 'module' });
        workers++;
      } catch {
        // No workers (tests, restricted contexts): build in place.
        const job = queue.shift()!;
        try { job.resolve(finishBlas(job.corners, job.shading)); } catch (e) { job.reject(e); }
        continue;
      }
    }
    if (!worker) return;
    const job = queue.shift()!;
    const w = worker;
    w.onmessage = (e: MessageEvent) => { idle.push(w); job.resolve(e.data as BlasData); pump(); };
    w.onerror = (e) => { workers--; w.terminate(); job.reject(e); pump(); };
    w.postMessage({ corners: job.corners, shading: job.shading }, [job.corners.buffer, job.shading.buffer]);
  }
}

/** Build in the worker pool. The arrays are transferred (unusable afterwards). */
export function finishBlasInWorker(corners: Float32Array, shading: Float32Array): Promise<BlasData> {
  if (typeof Worker === 'undefined') return Promise.resolve(finishBlas(corners, shading));
  return new Promise((resolve, reject) => { queue.push({ corners, shading, resolve, reject }); pump(); });
}
