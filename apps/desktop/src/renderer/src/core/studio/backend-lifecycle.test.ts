import { flushStudioShutters } from './index';
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
 it('keeps the current native session after a slow quality frame or a long pause',async()=>{
  let now=0;const clock=vi.spyOn(performance,'now').mockImplementation(()=>now),dispose=vi.fn(),create=vi.fn();
  const source=document.createElement('canvas');
  registerStudioBackend(()=>{create();return {id:'native-test',initialize:async()=>{},setPreparedScene:()=>{},setView:()=>{},setTime:()=>{},render:async(r:any)=>({source,width:64,height:64,samples:r.samples,version:'quality'}),dispose} as unknown as StudioBackend;});
  const {PM,draw}=fixture();
  try{
   draw();await studioIdle(PM);draw();await studioIdle(PM);
   now=15_000;expect(draw()?.canvas).toBe(source);
   draw(1);await studioIdle(PM);expect(draw(1)?.canvas).toBe(source);
   expect(create).toHaveBeenCalledOnce();expect(dispose).not.toHaveBeenCalled();
  }finally{disposeSceneRuntimes(PM);clock.mockRestore();}
 });
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
  expect(PM.Studio.hasUnpresentedFrame()).toBe(true);
  expect(draw(100)?.version).toContain('completed-during-drag');
  expect(PM.Studio.hasUnpresentedFrame()).toBe(false);
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
 it('lets a side pane present its own image without forcing a duplicate main-view composite',async()=>{
  registerStudioBackend(()=>({id:'native-test',initialize:async()=>{},setPreparedScene:()=>{},setView:()=>{},setTime:()=>{},render:async()=>({source:document.createElement('canvas'),width:64,height:64,samples:1,version:'side'}),dispose:()=>{}} as unknown as StudioBackend));
  const {PM,runtime}=fixture(),draw=()=>studioLayer(PM,runtime,64,64,'',new Set(['model']),true,runtime.camera,undefined,0,{slot:'pane-0'});
  draw();await studioIdle(PM);draw();await studioIdle(PM);
  expect(PM.Studio.hasUnpresentedFrame()).toBe(false);
  expect(draw()?.version).toContain('side');disposeSceneRuntimes(PM);
 });
 it('releases the previous quality tier instead of occupying another native surface slot',async()=>{
  const releases={live:vi.fn(),final:vi.fn()};
  registerStudioBackend(()=>({id:'native-test',initialize:async()=>{},setPreparedScene:()=>{},setView:()=>{},setTime:()=>{},render:async(r:any)=>({source:document.createElement('canvas'),width:64,height:64,samples:r.samples,version:r.tier,release:releases[r.tier as keyof typeof releases]}),dispose:()=>{}} as unknown as StudioBackend));
  const {PM,draw}=fixture();draw();await studioIdle(PM);draw();await studioIdle(PM);expect(draw()?.version).toContain('final');
  PM._studioFinal=false;PM.store.get=()=> 'fast';draw();await studioIdle(PM);
  expect(releases.final).toHaveBeenCalledOnce();expect(draw()?.version).toContain('live');
  expect(releases.live).not.toHaveBeenCalled();disposeSceneRuntimes(PM);expect(releases.live).toHaveBeenCalledOnce();
 });
 it('waits for native initialization and async per-layer frames, then reuses the completed frame',async()=>{
  const requests:any[]=[],dispose=vi.fn(),source=document.createElement('canvas');
  registerStudioBackend(()=>({id:'native-test',initialize:async()=>{},setPreparedScene:async()=>{},setView:async()=>{},setTime:async()=>{},render:async(r:any)=>{requests.push(r);return {source,width:64,height:64,samples:r.samples,version:'native'};},dispose} as unknown as StudioBackend));
  const {PM,draw}=fixture();expect(draw()).toBeNull();await studioIdle(PM);expect(studioPending(PM)).toBe(true);
  expect(draw()).toBeNull();await studioIdle(PM);expect(draw()?.canvas).toBe(source);expect(studioPending(PM)).toBe(false);
  expect(requests).toHaveLength(1);expect(requests[0]).toMatchObject({tier:'final',layer:'model',background:true});
  registerStudioBackend(null);draw();expect(dispose).toHaveBeenCalledOnce();
 });
 it('uses bounded adaptive quality by default for exports and offscreen stills',async()=>{
  const requests:any[]=[];
  registerStudioBackend(()=>({id:'native-test',initialize:async()=>{},setPreparedScene:()=>{},setView:()=>{},setTime:()=>{},render:async(r:any)=>{requests.push(r);return {source:document.createElement('canvas'),width:64,height:64,samples:r.samples,version:'quality'};},dispose:()=>{}} as unknown as StudioBackend));
  const {PM,draw}=fixture();PM._studioShutterSamples=2;
  draw();await studioIdle(PM);draw();await studioIdle(PM);
  expect(requests[0]).toMatchObject({tier:'final',adaptive:true,timeBudgetMs:1500,temporal:false,minSamples:16,noiseThreshold:.03,maxBounces:8,caustics:true});
  expect(requests[0].samples).toBe(128);
  PM._studioFinal=false;PM._studioStill=true;
  draw(1);await studioIdle(PM);draw(1);await studioIdle(PM);
  expect(requests.at(-1)).toMatchObject({tier:'final',samples:256,adaptive:true,timeBudgetMs:3000,temporal:false});
  disposeSceneRuntimes(PM);
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

it('plans all combined shutter times before resolving and denoises one shared frame',async()=>{
  vi.stubGlobal('OffscreenCanvas',class{width:number;height:number;constructor(w:number,h:number){this.width=w;this.height=h;}getContext(){return {drawImage:vi.fn()};}});
  const render=vi.fn(),batch=vi.fn(async(samples:any[])=>({source:document.createElement('canvas'),width:64,height:64,samples:samples.reduce((n,s)=>n+s.request.samples,0),version:'integrated'}));
  registerStudioBackend(()=>({id:'native-test',shutterAccumulation:true,initialize:async()=>{},setPreparedScene:()=>{},setView:()=>{},setTime:()=>{},render,renderShutter:batch,dispose:()=>{}} as unknown as StudioBackend));
  const PM:any={proj:{layers:[{id:'a'},{id:'b'}]},time:0,_studioFinal:true,_studioPreparing:true,_studioShutterSamples:2,invalidate:vi.fn(),store:{get:()=> 'quality'}};
  const runtime=new SceneRuntime(),scene=createScene();scene.objects=[createObject('a'),createObject('b')];runtime.sync(scene);
  const draw=(time:number)=>studioLayer(PM,runtime,64,64,'a',new Set(['a','b']),true,runtime.camera,undefined,time);
  beginStudioCapture(PM);draw(0);await studioIdle(PM);
  draw(0);draw(.01);flushStudioShutters(PM);await studioIdle(PM);
  expect(batch).toHaveBeenCalledOnce();expect(batch.mock.calls[0]![0].map((s:any)=>s.time)).toEqual([0,.01]);
  expect(render).not.toHaveBeenCalled();expect(PM.Studio.status().declined).toBe('');PM._studioPreparing=false;
  expect(draw(0)?.canvas).toBeTruthy();expect(draw(.01)?.canvas).toBeTruthy();
  // Reusing one composition twice at distinct source times must not mix the
  // two instances into one four-stratum image.
  batch.mockClear();render.mockImplementation(async(r:any)=>({source:document.createElement('canvas'),width:64,height:64,samples:r.samples,version:'separate'}));
  PM._studioPreparing=true;beginStudioCapture(PM);
  for(const time of [0,.01,.02,.03])draw(time);flushStudioShutters(PM);await studioIdle(PM);
  expect(batch).not.toHaveBeenCalled();expect(render).toHaveBeenCalledTimes(4);
  disposeSceneRuntimes(PM);runtime.dispose();
});
