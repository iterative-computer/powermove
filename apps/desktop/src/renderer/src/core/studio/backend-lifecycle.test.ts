// @vitest-environment happy-dom
import * as THREE from 'three';
import { afterEach, describe, expect, it, vi } from 'vitest';
const fallback = vi.hoisted(()=>({renders:0,disposals:0}));
vi.mock('./gpu',()=>({initStudioGpu:async()=>true,studioGpuReady:()=>true,studioGpuFailure:()=>null,studioMaxBindingBytes:()=>1e9,studioOptions:{denoise:true,caustics:true}}));
vi.mock('./webgpu-backend',()=>({WebGpuStudioBackend:class{
 id='webgpu';async initialize(){}setPreparedScene(){}setView(){}setTime(){}
 render(r:any){fallback.renders++;return {source:document.createElement('canvas'),width:r.size.width,height:r.size.height,samples:r.samples,version:'fallback'};}
 dispose(){fallback.disposals++;}
}}));
import { registerStudioBackend, type StudioBackend } from '../scene3d/studio-backend';
import { SceneRuntime } from '../scene3d/runtime';
import { disposeSceneRuntimes } from '../scene3d/service';
import { createScene, createObject } from '../scene3d/schema';
import { beginStudioCapture,studioIdle, studioLayer, studioPending } from './index';
function fixture(){const PM:any={time:0,_studioFinal:true,invalidate:vi.fn(),store:{get:()=> 'quality'}};const runtime=new SceneRuntime();const scene=createScene();scene.objects=[createObject('model')];runtime.sync(scene);return {PM,runtime,draw:(time=0)=>studioLayer(PM,runtime,64,64,'model',new Set(['model']),true,runtime.camera,undefined,time)};}
afterEach(()=>{registerStudioBackend(null);vi.unstubAllGlobals();});
describe('Studio backend lifecycle',()=>{
 it('waits for native initialization and async per-layer frames, then reuses the completed frame',async()=>{
  const requests:any[]=[],dispose=vi.fn(),source=document.createElement('canvas');
  registerStudioBackend(()=>({id:'native-test',initialize:async()=>{},setPreparedScene:async()=>{},setView:async()=>{},setTime:async()=>{},render:async(r:any)=>{requests.push(r);return {source,width:64,height:64,samples:r.samples,version:'native'};},dispose} as unknown as StudioBackend));
  const {PM,draw}=fixture();expect(draw()).toBeNull();await studioIdle(PM);expect(studioPending(PM)).toBe(true);
  expect(draw()).toBeNull();await studioIdle(PM);expect(draw()?.canvas).toBe(source);expect(studioPending(PM)).toBe(false);
  expect(requests).toHaveLength(1);expect(requests[0]).toMatchObject({tier:'final',layer:'model',background:true});
  registerStudioBackend(null);draw();expect(dispose).toHaveBeenCalledOnce();
 });
 it('recovers a native initialization failure with in-page WebGPU',async()=>{
  const dispose=vi.fn();registerStudioBackend(()=>({id:'broken-native',initialize:async()=>{throw new Error('host unavailable');},dispose} as unknown as StudioBackend));
  const {PM,draw}=fixture();draw();await studioIdle(PM);draw();await studioIdle(PM);const frame=draw();
  expect(frame?.version).toContain('fallback');expect(dispose).toHaveBeenCalledOnce();expect(PM.Studio.status().backend).toBe('webgpu');
 });
 it('sends canonical Rust scenes to a backend without the optional binary fast path',async()=>{
  let description:any;registerStudioBackend(()=>({id:'json-test',initialize:async()=>{},setScene:(d:any)=>{description=d;},setCamera:()=>{},setTime:()=>{},render:(r:any)=>({source:document.createElement('canvas'),width:64,height:64,samples:r.samples,version:'json'}),dispose:()=>{}} as unknown as StudioBackend));
  const {PM,draw}=fixture();draw();await studioIdle(PM);draw();await studioIdle(PM);
  expect(description.version).toBe(1);expect(description.scene.objects[0].mesh.indices).toHaveLength(12);expect(description.layer_ids).toEqual(['model']);
 });
 it('releases renderer sessions and in-flight frames when a project closes',async()=>{
  const dispose=vi.fn(),release=vi.fn();let complete:any;
  registerStudioBackend(()=>({id:'native-test',initialize:async()=>{},setPreparedScene:()=>{},setView:()=>{},setTime:()=>{},render:()=>new Promise(resolve=>{complete=resolve;}),dispose} as unknown as StudioBackend));
  const {PM,draw}=fixture();draw();await studioIdle(PM);draw();
  disposeSceneRuntimes(PM);expect(dispose).toHaveBeenCalledOnce();
  complete({source:document.createElement('canvas'),width:64,height:64,samples:64,version:'late',release});
  await Promise.resolve();await Promise.resolve();expect(release).toHaveBeenCalledOnce();expect(studioPending(PM)).toBe(false);
 });
 it('keeps every async shutter sample ready for the final synchronous compositor',async()=>{
  vi.stubGlobal('OffscreenCanvas',class{width:number;height:number;constructor(w:number,h:number){this.width=w;this.height=h;}getContext(){return {drawImage:vi.fn()};}});
  const rendered:number[]=[];let time=0;registerStudioBackend(()=>({id:'native-test',initialize:async()=>{},setPreparedScene:()=>{},setView:()=>{},setTime:(t:number)=>{time=t;},render:async()=>{rendered.push(time);return {source:document.createElement('canvas'),width:64,height:64,samples:16,version:`time:${time}`};},dispose:()=>{}} as unknown as StudioBackend));
  const {PM,draw}=fixture();PM._studioShutterSamples=4;beginStudioCapture(PM);draw();await studioIdle(PM);
  draw(0);draw(.1);await studioIdle(PM);const a=draw(0),b=draw(.1);
  expect(rendered).toEqual([0,.1]);expect(a?.canvas).not.toBe(b?.canvas);
  expect(draw(0)?.canvas).toBe(a?.canvas);
  beginStudioCapture(PM);expect((a?.canvas as any).width).toBe(0);
 });
});
