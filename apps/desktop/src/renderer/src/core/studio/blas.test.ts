import { afterEach,expect,it,vi } from 'vitest';
import { finishGeometryInWorker,studioGeometryIdle } from './blas';
afterEach(()=>vi.unstubAllGlobals());
it('does not abort frame preparation when an individual mesh worker fails',async()=>{
  vi.stubGlobal('Worker',class {
    onerror?: (event:unknown)=>void;terminate=vi.fn();
    postMessage(){queueMicrotask(()=>this.onerror?.({message:'Invalid mesh'}));}
  });
  const pending=finishGeometryInWorker({attributes:{},index:null,groups:[],materials:[],object:0});
  const failure=expect(pending).rejects.toMatchObject({message:'Invalid mesh'});
  await expect(studioGeometryIdle()).resolves.toBeUndefined();await failure;
});
