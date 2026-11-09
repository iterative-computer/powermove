import {prepareEnvironment,type EnvironmentJob,type TerrainPart,type ScatterPart} from './environment-preparation';
type Job={input:EnvironmentJob;owner:object;bytes:number;resolve:(value:any)=>void;reject:(error:Error)=>void};
const queue:Job[]=[],pending=new Set<Promise<unknown>>();
let worker:Worker|null=null,active:Job|null=null,timer:ReturnType<typeof setTimeout>|undefined;
let retainedBytes=0;
function retire(){if(timer)clearTimeout(timer);timer=undefined;if(worker){worker.onmessage=null;worker.onerror=null;worker.terminate();worker=null;}}
function done(result?:unknown,error?:Error){const job=active;active=null;retainedBytes-=job?.bytes??0;if(timer)clearTimeout(timer);if(worker){worker.onmessage=null;worker.onerror=null;}if(error){retire();job?.reject(error);}else job?.resolve(result);if(queue.length)pump();else timer=setTimeout(retire,5000);}
function pump(){
 if(active||!queue.length)return;if(timer)clearTimeout(timer);
 try{worker??=new Worker(new URL('./environment.worker.ts',import.meta.url),{type:'module'});}
 catch(error){const job=queue.shift()!;retainedBytes-=job.bytes;job.reject(new Error(String(error)));pump();return;}
 active=queue.shift()!;worker.onmessage=e=>done(e.data.result,e.data.error?new Error(e.data.error):undefined);worker.onerror=e=>done(undefined,new Error(e.message));
 timer=setTimeout(()=>done(undefined,new Error('Environment preparation timed out')),60000);
 const transfers:Transferable[]=[];
 if(active.input.kind==='scatter')for(const s of active.input.surfaces){for(const a of Object.values(s.geometry.attributes))transfers.push(a.array.buffer as ArrayBuffer);if(s.geometry.index)transfers.push(s.geometry.index.buffer as ArrayBuffer);}
 try{worker.postMessage(active.input,[...new Set(transfers)]);}catch(error){done(undefined,new Error(String(error)));}
}
export function prepareEnvironmentInWorker<T extends TerrainPart[]|ScatterPart[]>(input:EnvironmentJob,owner:object):Promise<T>{
 if(typeof Worker==='undefined')return Promise.resolve(prepareEnvironment(input) as T);
 // Replace queued edits for the same terrain/scatter rather than retaining
 // every intermediate variation while a previous generation is running.
 for(let i=queue.length-1;i>=0;i--)if(queue[i]!.owner===owner){const job=queue.splice(i,1)[0]!;retainedBytes-=job.bytes;job.reject(new Error('Environment preparation superseded'));}
 if(queue.length>=8)return Promise.reject(new Error('Too many pending environment preparations'));
 const bytes=input.kind==='terrain'?(input.settings.resolution+1)**2*1024+(input.settings.heights?.length??0)*8:input.surfaces.reduce((n,s)=>n+Object.values(s.geometry.attributes).reduce((n,a)=>n+a.array.byteLength,s.geometry.index?.byteLength??0),0)+input.settings.count*512;
 if(!Number.isFinite(bytes)||retainedBytes+bytes>512*1024**2)return Promise.reject(new Error('Environment preparation exceeds the memory budget'));
 retainedBytes+=bytes;
 const promise=new Promise<T>((resolve,reject)=>{queue.push({input,owner,bytes,resolve,reject});pump();}).finally(()=>pending.delete(promise));pending.add(promise);return promise;
}
export async function environmentPreparationIdle(){while(pending.size)await Promise.allSettled([...pending]);}
export function disposeEnvironmentWorkers(){retire();active?.reject(new Error('Studio project closed'));active=null;for(const job of queue)job.reject(new Error('Studio project closed'));queue.length=0;retainedBytes=0;}
