import { afterEach, expect, it, vi } from 'vitest';
afterEach(()=>{vi.unstubAllGlobals();vi.resetModules();});

it('ignores shader completions and errors from a lost device after a replacement is ready',async()=>{
  let lose!:()=>void,finishShaders!:(value:unknown)=>void;
  const lost=new Promise<void>(resolve=>{lose=resolve;}),shaders=new Promise(resolve=>{finishShaders=resolve;});
  const callbacks:((event:any)=>void)[]=[];
  const makeDevice=(id:number)=>({
    lost:id===1?lost:new Promise(()=>{}),
    addEventListener:(_name:string,fn:(event:any)=>void)=>callbacks.push(fn),
    createBindGroupLayout:()=>({id}),createPipelineLayout:()=>({id}),createShaderModule:()=>({id}),createSampler:()=>({id}),
    createComputePipelineAsync:vi.fn(()=>id===1?shaders:Promise.resolve({id})),
    createRenderPipelineAsync:()=>id===1?shaders:Promise.resolve({id}),
  });
  const first=makeDevice(1),second=makeDevice(2),requestDevice=vi.fn().mockResolvedValueOnce(first).mockResolvedValueOnce(second);
  vi.stubGlobal('navigator',{gpu:{requestAdapter:async()=>({limits:{maxStorageBuffersPerShaderStage:12,maxStorageBufferBindingSize:1e9,maxBufferSize:1e9},requestDevice}),getPreferredCanvasFormat:()=> 'bgra8unorm'}});
  vi.stubGlobal('GPUShaderStage',{COMPUTE:4,FRAGMENT:2});
  const api=await import('./gpu'),initializing=api.initStudioGpu();
  await vi.waitFor(()=>expect(first.createComputePipelineAsync).toHaveBeenCalled());
  lose();await Promise.resolve();
  await expect(api.initStudioGpu()).resolves.toBe(second);
  finishShaders({id:1});await expect(initializing).resolves.toBeNull();
  callbacks[0]!({error:new Error('Old device error')});
  expect(api.studioGpuReady()).toBe(true);expect(api.studioGpuFailure()).toBeNull();
});
