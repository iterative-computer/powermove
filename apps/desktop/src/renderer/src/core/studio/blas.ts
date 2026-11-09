import { gatherPayload, type GeometryPayload } from './gather';
import { buildBvh, type Bvh } from './bvh';
import { STUDIO_TRIANGLE_FLOATS } from './layout';
import {environmentPreparationIdle,disposeEnvironmentWorkers} from '../scene3d/environment-worker';

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

type Payload={corners:Float32Array;shading:Float32Array}|{geometry:GeometryPayload}|{top:Float32Array};
export type GeometryBlas=BlasData & {bounds:number[]};
type Job={payload:Payload;bytes:number;resolve:(b:any)=>void;reject:(e:unknown)=>void};
export function prepareBlasPayload(payload:Payload):BlasData|GeometryBlas|Bvh {
 if('top' in payload)return buildBvh(payload.top);
 if('geometry' in payload){const g=gatherPayload(payload.geometry);return {...finishBlas(g.corners,g.shading),bounds:[...g.bounds.min.toArray(),...g.bounds.max.toArray()]};}
 return finishBlas(payload.corners,payload.shading);
}
const queue: Job[] = [];
const idle = new Map<Worker, ReturnType<typeof setTimeout>>();
const active = new Map<Worker, {job:Job;timer:ReturnType<typeof setTimeout>}>();
const workers = new Set<Worker>();
let retainedBytes = 0;
const WORKER_BYTES = 512 * 1024 ** 2;
const pendingBuilds=new Set<Promise<unknown>>();
function track<T>(promise:Promise<T>):Promise<T>{const pending=promise.finally(()=>pendingBuilds.delete(pending));pendingBuilds.add(pending);return pending;}
export async function studioGeometryIdle():Promise<void>{await environmentPreparationIdle();await Promise.allSettled([...pendingBuilds]);}
const maxWorkers = Math.max(1, Math.min(4, ((globalThis as any).navigator?.hardwareConcurrency ?? 4) - 1));

function retire(worker:Worker):void {
  const timer=idle.get(worker);if(timer)clearTimeout(timer);idle.delete(worker);
  worker.onmessage=null;worker.onerror=null;worker.terminate();workers.delete(worker);
}
/** The pool belongs to this renderer page. Project close cancels its retained
 * inputs and callbacks; the next project starts a fresh bounded pool. */
export function disposeStudioGeometryWorkers():void {
  disposeEnvironmentWorkers();
  for(const {job,timer} of active.values()){clearTimeout(timer);job.reject(new Error('Studio project closed'));}
  for(const job of queue)job.reject(new Error('Studio project closed'));
  queue.length=0;active.clear();retainedBytes=0;
  for(const worker of [...workers])retire(worker);
}
function finish(worker:Worker,result?:unknown,error?:unknown):void {
  const running=active.get(worker);if(!running)return;
  clearTimeout(running.timer);active.delete(worker);retainedBytes-=running.job.bytes;
  // Resolved promises can retain their large result. Never leave a completed
  // job's resolve closure installed on an idle worker.
  worker.onmessage=null;worker.onerror=null;
  if(error){retire(worker);running.job.reject(error);}
  else {idle.set(worker,setTimeout(()=>retire(worker),5000));running.job.resolve(result);}
  pump();
}
function pump(): void {
  while(queue.length){
    let worker=idle.keys().next().value as Worker|undefined;
    if(worker){clearTimeout(idle.get(worker));idle.delete(worker);}
    if(!worker && workers.size<maxWorkers){
      try {worker=new Worker(new URL('./blas.worker.ts',import.meta.url),{type:'module'});workers.add(worker);}
      catch(error){const job=queue.shift()!;retainedBytes-=job.bytes;job.reject(error);continue;}
    }
    if(!worker)return;
    const job=queue.shift()!,w=worker;
    const timer=setTimeout(()=>finish(w,undefined,new Error('Studio geometry preparation timed out')),30_000);
    active.set(w,{job,timer});
    w.onmessage=e=>finish(w,e.data);
    w.onerror=e=>finish(w,undefined,e);
    const payload=job.payload,transfers='top' in payload?[payload.top.buffer]:'geometry' in payload?[...Object.values(payload.geometry.attributes).map(a=>a.array.buffer),...(payload.geometry.index?[payload.geometry.index.buffer]:[])]:[payload.corners.buffer,payload.shading.buffer];
    try {w.postMessage(payload,[...new Set(transfers)]);}
    catch(error){finish(w,undefined,error);}
  }
}
function enqueue<T>(payload:Payload):Promise<T>{
  const geometry='geometry' in payload?payload.geometry:null;
  const input='top' in payload?payload.top.byteLength:geometry?Object.values(geometry.attributes).reduce((n,a)=>n+a.array.byteLength,geometry.index?.byteLength??0):('corners' in payload?payload.corners.byteLength+payload.shading.byteLength:0);
  const triangles=geometry?(geometry.index?.length??((geometry.attributes.position?.array.length??0)/3))/3:('corners' in payload?payload.corners.length/9:0);
  const bytes=input+('top' in payload?payload.top.length/9*320:triangles*320);
  if(!Number.isFinite(bytes) || retainedBytes+bytes>WORKER_BYTES)return Promise.reject(new Error('Studio geometry preparation exceeds the memory budget; simplify the model'));
  retainedBytes+=bytes;
  return track(new Promise((resolve,reject)=>{queue.push({payload,bytes,resolve,reject});pump();}));
}
/** Build in the worker pool. The arrays are transferred (unusable afterwards). */
export function finishBlasInWorker(corners: Float32Array, shading: Float32Array): Promise<BlasData> {
  if(typeof Worker==='undefined')return Promise.resolve(finishBlas(corners,shading));
  return enqueue({corners,shading});
}
/** Gather triangles and build their BVH without blocking editor input. */
export function finishGeometryInWorker(geometry:GeometryPayload):Promise<GeometryBlas>{
  if(typeof Worker==='undefined')return Promise.resolve(prepareBlasPayload({geometry}) as GeometryBlas);
  return enqueue({geometry});
}
/** A dense scatter's TLAS is preparation work too, never an editor task. */
export function finishTopInWorker(top:Float32Array):Promise<Bvh>{
 if(typeof Worker==='undefined')return Promise.resolve(buildBvh(top));
 return enqueue({top});
}
