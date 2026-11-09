import { afterEach,expect,it,vi } from 'vitest';
import { finishGeometryInWorker,studioGeometryIdle,disposeStudioGeometryWorkers } from './blas';
afterEach(()=>{disposeStudioGeometryWorkers();vi.useRealTimers();vi.unstubAllGlobals();});
it('does not abort frame preparation when an individual mesh worker fails',async()=>{
  vi.stubGlobal('Worker',class {
    onerror?: (event:unknown)=>void;terminate=vi.fn();
    postMessage(){queueMicrotask(()=>this.onerror?.({message:'Invalid mesh'}));}
  });
  const pending=finishGeometryInWorker({attributes:{},index:null,groups:[],materials:[],object:0});
  const failure=expect(pending).rejects.toMatchObject({message:'Invalid mesh'});
  await expect(studioGeometryIdle()).resolves.toBeUndefined();await failure;
});
it('releases worker handlers and reaps idle workers after completing geometry',async()=>{
  vi.useFakeTimers();
  let worker:any;
  vi.stubGlobal('Worker',class {
    onmessage:any;onerror:any;terminate=vi.fn();
    constructor(){worker=this;}
    postMessage(){queueMicrotask(()=>this.onmessage?.({data:{positions:new Float32Array(),tris:new Float32Array(),nodes:new ArrayBuffer(64),triangles:0,bounds:[]}}));}
  });
  await finishGeometryInWorker({attributes:{},index:null,groups:[],materials:[],object:0});
  expect(worker.onmessage).toBeNull();expect(worker.onerror).toBeNull();
  await vi.advanceTimersByTimeAsync(5000);
  expect(worker.terminate).toHaveBeenCalledOnce();
});
it('cancels in-flight geometry when its renderer closes and ignores late replies',async()=>{
  let worker:any;
  vi.stubGlobal('Worker',class {onmessage:any;onerror:any;terminate=vi.fn();constructor(){worker=this;}postMessage(){} });
  const pending=finishGeometryInWorker({attributes:{},index:null,groups:[],materials:[],object:0});
  const result=expect(pending).rejects.toThrow('closed');
  disposeStudioGeometryWorkers();await result;
  expect(worker.terminate).toHaveBeenCalledOnce();expect(worker.onmessage).toBeNull();
  await expect(studioGeometryIdle()).resolves.toBeUndefined();
});
it('rejects transfer failures without retaining a busy worker or hanging readiness',async()=>{
  vi.stubGlobal('Worker',class {terminate=vi.fn();postMessage(){throw new Error('Transfer failed');} });
  const failed=expect(finishGeometryInWorker({attributes:{},index:null,groups:[],materials:[],object:0})).rejects.toThrow('Transfer failed');
  await failed;await expect(studioGeometryIdle()).resolves.toBeUndefined();
});
