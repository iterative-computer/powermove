import { gatherPayload, type GeometryPayload } from './gather';
import { buildBvh } from './bvh';
import { STUDIO_TRIANGLE_FLOATS } from './layout';

/*
 * One mesh's bottom-level BVH: its triangles' corners and shading data in BVH
 * order. Building it is the slow part of preparing a large model, so it runs
 * in a pool of workers (blas.worker.ts) while the editor keeps drawing with
 * the classic renderer; exports build in place.
 */

export type BlasData = { positions: Float32Array; tris: Float32Array; nodes: ArrayBuffer; triangles: number };

/** `corners`: 9 floats per triangle; `shading`: 36 floats per triangle. */
export function finishBlas(corners: Float32Array, shading: Float32Array): BlasData {
  const triangles = corners.length / 9;
  if (!triangles) return { positions: new Float32Array(0), tris: new Float32Array(0), nodes: new ArrayBuffer(64), triangles: 0 };
  const { nodes, order } = buildBvh(corners);
  const positions = new Float32Array(triangles * 12);
  const tris = new Float32Array(triangles * STUDIO_TRIANGLE_FLOATS);
  for (let i = 0; i < triangles; i++) {
    const t = order[i]!;
    for (let c = 0; c < 3; c++) {
      positions[i * 12 + c * 4] = corners[t * 9 + c * 3]!;
      positions[i * 12 + c * 4 + 1] = corners[t * 9 + c * 3 + 1]!;
      positions[i * 12 + c * 4 + 2] = corners[t * 9 + c * 3 + 2]!;
    }
    tris.set(shading.subarray(t * STUDIO_TRIANGLE_FLOATS, t * STUDIO_TRIANGLE_FLOATS + STUDIO_TRIANGLE_FLOATS), i * STUDIO_TRIANGLE_FLOATS);
  }
  return { positions, tris, nodes, triangles };
}

/** Meshes at least this large build in a worker; smaller ones build at once. */
export const WORKER_TRIANGLES = 20_000;

type Payload={corners:Float32Array;shading:Float32Array}|{geometry:GeometryPayload};
export type GeometryBlas=BlasData & {bounds:number[]};
type Job={payload:Payload;resolve:(b:any)=>void;reject:(e:unknown)=>void};
export function prepareBlasPayload(payload:Payload):BlasData|GeometryBlas {
 if('geometry' in payload){const g=gatherPayload(payload.geometry);return {...finishBlas(g.corners,g.shading),bounds:[...g.bounds.min.toArray(),...g.bounds.max.toArray()]};}
 return finishBlas(payload.corners,payload.shading);
}
const queue: Job[] = [];
const idle: Worker[] = [];
let workers = 0;
const pendingBuilds=new Set<Promise<unknown>>();
function track<T>(promise:Promise<T>):Promise<T>{const pending=promise.finally(()=>pendingBuilds.delete(pending));pendingBuilds.add(pending);return pending;}
export async function studioGeometryIdle():Promise<void>{await Promise.allSettled([...pendingBuilds]);}

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
        try { job.resolve(prepareBlasPayload(job.payload)); } catch (e) { job.reject(e); }
        continue;
      }
    }
    if (!worker) return;
    const job = queue.shift()!;
    const w = worker;
    w.onmessage = (e: MessageEvent) => { idle.push(w); job.resolve(e.data as BlasData); pump(); };
    w.onerror = (e) => { workers--; w.terminate(); job.reject(e); pump(); };
    const payload=job.payload,transfers='geometry' in payload?[...Object.values(payload.geometry.attributes).map(a=>a.array.buffer),...(payload.geometry.index?[payload.geometry.index.buffer]:[])]:[payload.corners.buffer,payload.shading.buffer];
    w.postMessage(payload,transfers);
  }
}

/** Build in the worker pool. The arrays are transferred (unusable afterwards). */
export function finishBlasInWorker(corners: Float32Array, shading: Float32Array): Promise<BlasData> {
  if (typeof Worker === 'undefined') return Promise.resolve(finishBlas(corners, shading));
  return track(new Promise((resolve, reject) => { queue.push({ payload:{corners,shading}, resolve, reject }); pump(); }));
}

/** Gather triangles and build their BVH in a worker, so large imports never block input. */
export function finishGeometryInWorker(geometry:GeometryPayload):Promise<GeometryBlas> {
 if(typeof Worker==='undefined')return Promise.resolve(prepareBlasPayload({geometry}) as GeometryBlas);
 return track(new Promise((resolve,reject)=>{queue.push({payload:{geometry},resolve,reject});pump();}));
}
