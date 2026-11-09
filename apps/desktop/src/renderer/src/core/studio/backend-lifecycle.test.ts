// @vitest-environment happy-dom
import * as THREE from 'three';
import { afterEach, describe, expect, it, vi } from 'vitest';
const fallback = vi.hoisted(()=>({renders:0,disposals:0,generation:1,ready:true,initializations:0}));
vi.mock('./gpu',()=>({initStudioGpu:async()=>{fallback.initializations++;fallback.ready=true;return true;},studioGpuReady:()=>fallback.ready,studioGpuGeneration:()=>fallback.generation,studioGpuFailure:()=>null,studioMaxBindingBytes:()=>1e9,studioOptions:{denoise:true,caustics:true}}));
vi.mock('./webgpu-backend',()=>({WebGpuStudioBackend:class{
 id='webgpu';async initialize(){}setPreparedScene(){}setView(){}setTime(){}
 render(r:any){fallback.renders++;return {source:document.createElement('canvas'),width:r.size.width,height:r.size.height,samples:r.samples,version:'fallback'};}
 dispose(){fallback.disposals++;}
}}));
import { registerStudioBackend, type StudioBackend } from '../scene3d/studio-backend';
import { SceneRuntime } from '../scene3d/runtime';
import { disposeSceneRuntimes } from '../scene3d/service';
import { createScene, createObject } from '../scene3d/schema';
import { beginStudioCapture,beginStudioCapturePass,studioIdle, studioLayer, studioPending } from './index';
function fixture(){const PM:any={time:0,_studioFinal:true,invalidate:vi.fn(),store:{get:()=> 'quality'}};const runtime=new SceneRuntime();const scene=createScene();scene.objects=[createObject('model')];runtime.sync(scene);return {PM,runtime,draw:(time=0)=>studioLayer(PM,runtime,64,64,'model',new Set(['model']),true,runtime.camera,undefined,time)};}
afterEach(()=>{registerStudioBackend(null);vi.unstubAllGlobals();});
describe('Studio backend lifecycle',()=>{
 it('clears preparation for a deleted depth set after importing or removing a model',async()=>{
  const {PM,runtime,draw}=fixture();runtime.geometryPending=1;draw();expect(PM.Studio.status().preparing).toBeTruthy();
  const scene=createScene();scene.objects=[createObject('other')];runtime.geometryPending=0;runtime.sync(scene);
  studioLayer(PM,runtime,64,64,'other',new Set(['other']),true,runtime.camera);await studioIdle(PM);
  studioLayer(PM,runtime,64,64,'other',new Set(['other']),true,runtime.camera);
  expect(PM.Studio.status().preparing).toBeNull();disposeSceneRuntimes(PM);
 });
 it('coalesces Live input while displaying completed frames during a continuous drag',async()=>{
  const requests:any[]=[],release=vi.fn();let complete:any;
  registerStudioBackend(()=>({id:'native-test',initialize:async()=>{},setPreparedScene:()=>{},setView:()=>{},setTime:()=>{},render:(r:any)=>{requests.push(r);return new Promise(resolve=>{complete=resolve;});},dispose:()=>{}} as unknown as StudioBackend));
  const {PM,draw}=fixture();PM._studioFinal=false;PM.store.get=()=> 'fast';
  draw();await studioIdle(PM);draw(0);
  for(let time=1;time<=100;time++)draw(time);
  expect(requests).toHaveLength(1);
  complete({source:document.createElement('canvas'),width:64,height:64,samples:1,version:'completed-during-drag',release});
  await vi.waitFor(()=>expect(requests).toHaveLength(2));
  expect(draw(100)?.version).toContain('completed-during-drag');
  expect(release).not.toHaveBeenCalled();
  disposeSceneRuntimes(PM);
  complete({source:document.createElement('canvas'),width:64,height:64,samples:1,version:'closed',release});
 });
 it('restarts a lost portable device and replaces stale completed frames',async()=>{
  registerStudioBackend(null);
  const {PM,draw}=fixture();draw();await studioIdle(PM);draw();await studioIdle(PM);expect(draw()).toBeTruthy();
  const before=fallback.disposals,frames=fallback.renders;
  fallback.generation++;fallback.ready=false;
  draw();await Promise.resolve();await Promise.resolve();
  draw();await studioIdle(PM);draw();await studioIdle(PM);
  expect(fallback.disposals).toBeGreaterThan(before);expect(fallback.renders).toBeGreaterThan(frames);
  disposeSceneRuntimes(PM);
 });
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
  PM._studioFinal=false;draw(.2);PM._studioFinal=true;PM._studioPreparing=true;
  beginStudioCapturePass(PM);draw(0);draw(.1);expect(studioPending(PM)).toBe(false);
  PM._studioPreparing=false;
  beginStudioCapture(PM);expect((a?.canvas as any).width).toBe(0);
 });
 it('requests native Live at output resolution and releases superseded depth sets',async()=>{
  const requests:any[]=[],dispose=vi.fn();
  registerStudioBackend(()=>({id:'native-test',initialize:async()=>{},setPreparedScene:()=>{},setView:()=>{},setTime:()=>{},render:async(r:any)=>{requests.push(r);return {source:document.createElement('canvas'),width:r.size.width,height:r.size.height,samples:1,version:'live'};},dispose} as unknown as StudioBackend));
  const {PM,runtime,draw}=fixture();PM._studioFinal=false;draw();await studioIdle(PM);draw();await studioIdle(PM);
  expect(requests[0]).toMatchObject({tier:'live',size:{width:64,height:64}});
  const scene=createScene();scene.objects=[createObject('model'),createObject('other')];runtime.sync(scene);
  studioLayer(PM,runtime,64,64,'model',new Set(['model','other']),true,runtime.camera);await studioIdle(PM);
  expect(dispose).toHaveBeenCalledOnce();
 });
 it('releases old combined-layer frames used by legacy mesh extensions',async()=>{
  const release=vi.fn();
  registerStudioBackend(()=>({id:'native-test',initialize:async()=>{},setPreparedScene:()=>{},setView:()=>{},setTime:()=>{},render:async()=>({source:document.createElement('canvas'),width:64,height:64,samples:64,version:'combined',release}),dispose:()=>{}} as unknown as StudioBackend));
  const {PM,runtime}=fixture(),draw=(time:number)=>studioLayer(PM,runtime,64,64,'',new Set(['model']),true,runtime.camera,undefined,time);
  draw(0);await studioIdle(PM);draw(0);await studioIdle(PM);draw(1);await studioIdle(PM);
  expect(release).toHaveBeenCalledOnce();
 });
});
