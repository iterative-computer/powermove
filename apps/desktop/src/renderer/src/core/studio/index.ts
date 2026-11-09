import { attachStudioWorld } from '../scene3d/environment-render';
import { combineOpaqueDepthSet } from './compositing';
import { studioGeometryIdle } from './blas';
import * as THREE from 'three';
import { extractScene, isPending, MAP_SIZE, type StudioPending, type StudioScene } from './extract';
import { initStudioGpu, studioGpuFailure, studioGpuReady, studioMaxBindingBytes, studioOptions, type StudioView } from './gpu';
import { studioSplatLayer } from './splat-raster';
import { WebGpuStudioBackend } from './webgpu-backend';
import { describeCamera, describePreparedScene } from './scene-description';
import { studioBackendFactory, studioBackendRevision, type StudioBackend, type StudioFrame, type StudioRenderRequest, type StudioTextureSource } from '../scene3d/studio-backend';

export const STUDIO_FINAL_SAMPLES = 64;
let emptyCombinedLayer:OffscreenCanvas|undefined;
const STUDIO_STILL_SAMPLES = 4, REFINE_SAMPLES = 256, REFINE_DELAY_MS = 140, JOB_TTL_MS = 10_000;
export type StudioPreview = 'fast' | 'quality';
type Job = {
  runtime:any; uid:number; depthIds:Set<string>; separate:boolean;sceneKey: string; viewKey: string; scene: StudioScene; maps: StudioTextureSource[];
  backend: StudioBackend; ready: boolean; disposed:boolean; chain: Promise<void> | null;
  frames: Map<string, StudioFrame>; pending: Map<string, Promise<void>>;
  screens: { layer: number; comp: string }[]; screenKey: string;
  changedAt: number; usedAt: number; refining: boolean; samples: number; refineGoal:number; generation: number;
  factoryRevision: number; completedGeneration: number; failure: string | null;
};
type Capture = { frames:Map<string,StudioFrame & {backendId?:string}>;requested:Set<string>;completed:Set<string>;loading:boolean };
type State = { jobs: Map<string, Job>; tick: number | null; enabled: boolean; preview: StudioPreview; previewListeners: Set<(q: StudioPreview) => void>; frames: number;backend:string;frameSamples:number; declined: string; revision: number; preparing: Map<string, StudioPending>; capture:Capture|null };
const states = new WeakMap<object, State>();
const state = (PM: any): State => {
  let s = states.get(PM);
  if (!s) {
    s = { jobs: new Map(), tick: null, enabled: true, preview: PM.store?.get?.('studioPreview', 'quality') === 'fast' ? 'fast' : 'quality', previewListeners: new Set(), frames: 0,backend:'webgpu',frameSamples:0, declined: '', revision: 0, preparing: new Map(),capture:null };
    states.set(PM, s); const st = s;
    PM.Studio ??= {
      setEnabled: (on: boolean) => setStudioEnabled(PM, on),
      setCaustics: (on: boolean) => { studioOptions.caustics = on; st.revision++; pruneStudio(PM, () => false); PM.invalidate?.('render'); },
      setDenoise: (on: boolean) => { studioOptions.denoise = on; st.revision++; pruneStudio(PM, () => false); PM.invalidate?.('render'); },
      getPreview: () => st.preview, setPreview: (q: StudioPreview) => setStudioPreview(PM, q), progress: () => studioProgress(PM),
      status: () => ({ enabled: st.enabled, ready: studioBackendFactory() ? [...st.jobs.values()].some(j => j.ready) : studioGpuReady(), failure: [...st.jobs.values()].find(j => j.failure)?.failure ?? studioGpuFailure(), frames: st.frames,frameSamples:st.frameSamples, declined: st.declined, preparing: preparingOf(st), backend:st.backend }),
      whenReady: () => studioReady(PM), idle: () => studioIdle(PM),
      liveHistory:async()=>{await studioIdle(PM);for(const job of st.jobs.values()){const stats=await (job.backend as any).liveHistory?.();if(stats)return stats;}return null;},
    };
    void initStudioGpu().then(ok => { if (ok) PM.invalidate?.('render'); });
  }
  return s;
};
export async function studioReady(PM: any): Promise<void> {
  state(PM);
  if (!studioBackendFactory() && !await initStudioGpu()) throw new Error(studioGpuFailure() || 'Studio could not start');
}
export function studioEnabled(PM: any): boolean { return state(PM).enabled && (studioGpuReady() || !!studioBackendFactory()); }
export function setStudioEnabled(PM: any, on: boolean): void { const s = state(PM); s.enabled = on; s.revision++; PM.invalidate?.('render'); }
export function studioPreview(PM: any): StudioPreview { return state(PM).preview; }
export function setStudioPreview(PM: any, q: StudioPreview): void {
  const s = state(PM); if (s.preview === q) return; s.preview = q; s.revision++; PM.store?.set?.('studioPreview', q);
  for (const listener of s.previewListeners) listener(q); PM.invalidate?.('render');
}
export function onStudioPreview(PM: any, listener: (q: StudioPreview) => void): () => void { const s = state(PM); s.previewListeners.add(listener); return () => s.previewListeners.delete(listener); }
export function studioPreviewKey(PM: any): string { const s = states.get(PM); return `${s?.enabled}:${s?.preview}:${s?.revision}:${studioBackendRevision()}`; }
export function studioUnsettled(PM: any): boolean { const s = states.get(PM); return !!s && [...s.jobs.values()].some(j => j.pending.size || s.preview === 'quality' && j.samples < REFINE_SAMPLES); }
function preparingOf(s: State): StudioPending | null { if (!s.preparing.size) return null; return [...s.preparing.values()].reduce((a, p) => ({ pending: a.pending + p.pending, total: a.total + p.total }), { pending: 0, total: 0 }); }
export function studioProgress(PM: any): { samples: number; goal: number; refining: boolean; preparing?: StudioPending } | null {
  const s = state(PM), preparing = preparingOf(s); if (preparing) return { samples: 0, goal: REFINE_SAMPLES, refining: false, preparing };
  if (s.preview === 'fast') return null; for (const j of s.jobs.values()) return { samples: j.samples, goal: REFINE_SAMPLES, refining: j.refining }; return null;
}
export function mapCanvas(source: any, flipY: boolean): OffscreenCanvas {
  const c = new OffscreenCanvas(MAP_SIZE, MAP_SIZE);
  {
    const ctx = c.getContext('2d')!;
    // three uploads flipY textures bottom row first; match its UVs.
    if (flipY) { ctx.translate(0, MAP_SIZE); ctx.scale(1, -1); }
    if (source.data && source.width && source.height) {
      const image = new OffscreenCanvas(source.width, source.height);
      const data = source.data instanceof Uint8ClampedArray ? source.data : new Uint8ClampedArray(source.data);
      image.getContext('2d')!.putImageData(new ImageData(data, source.width, source.height), 0, 0);
      ctx.drawImage(image, 0, 0, MAP_SIZE, MAP_SIZE);
    } else ctx.drawImage(source, 0, 0, MAP_SIZE, MAP_SIZE);
  }
  return c;
}

export function cameraOf(camera: THREE.Camera, w: number, h: number): StudioView['camera'] | null {
  const pc = camera as THREE.PerspectiveCamera;
  const oc = camera as THREE.OrthographicCamera;
  if (!pc.isPerspectiveCamera && !oc.isOrthographicCamera) return null;
  camera.updateMatrixWorld(true);
  const e = camera.matrixWorld.elements;
  const right = new THREE.Vector3(e[0], e[1], e[2]).normalize();
  const up = new THREE.Vector3(e[4], e[5], e[6]).normalize();
  const forward = new THREE.Vector3(-e[8], -e[9], -e[10]).normalize();
  return {
    position: [e[12], e[13], e[14]],
    forward: [forward.x, forward.y, forward.z],
    right: [right.x, right.y, right.z],
    up: [up.x, up.y, up.z],
    tanY: oc.isOrthographicCamera ? (oc.top - oc.bottom) / (2 * Math.max(oc.zoom, 1e-3)) : Math.tan(THREE.MathUtils.degToRad(pc.fov) / 2) / Math.max(pc.zoom, 1e-3),
    orthographic: !!oc.isOrthographicCamera, near: pc.near, far: pc.far,
    aspect: w / Math.max(1, h),
    lensRadius: 0,
    focus: 1,
  };
}

/** The studio backdrop when a set is on and no background color was chosen. */
export const STUDIO_BACKDROP = '#F6F5F8';

export function viewOf(runtime: any, camera: StudioView['camera'], bounds: THREE.Box3): StudioView {
  const raw = runtime.raw;
  const env = raw?.environment;
  const channel = (node: any, prefix: string, key: string, fallback: any) => {
    const prop = node?.p?.[key];
    if (!prop) return fallback;
    try { return runtime.evaluation(prop, `${prefix}.${key}`) ?? prop.v; } catch { return prop.v; }
  };
  const value = (key: string, fallback: any) => channel(env, 'environment', key, fallback);
  // The lens: an f-number opens the aperture; focus defaults to the aim point.
  camera.iris=[Number(channel(raw?.camera,'camera','irisShape',0)),Number(channel(raw?.camera,'camera','irisRotation',0))*Math.PI/180,Number(channel(raw?.camera,'camera','irisRoundness',0)),Number(channel(raw?.camera,'camera','irisAspect',1))];
  const fStop = Number(channel(raw?.camera, 'camera', 'fStop', 0));
  const focus = Number(channel(raw?.camera, 'camera', 'focusDistance', 0));
  const fov = Number(channel(raw?.camera, 'camera', 'fov', 45));
  if (fStop > 0) {
    // Focal length from the field of view on a 24 mm-tall sensor, in scene units
    // scaled like the scene (1 unit ≈ 100 mm at product scale).
    const focalMm = 12 / Math.tan(THREE.MathUtils.degToRad(fov) / 2);
    camera.lensRadius = focalMm*.001 / (2 * fStop * (runtime.metersPerUnit ?? .1));
  }
  const target = raw?.camera ? new THREE.Vector3(Number(channel(raw.camera, 'camera', 'targetX', 0)), Number(channel(raw.camera, 'camera', 'targetY', 0)), Number(channel(raw.camera, 'camera', 'targetZ', 0))) : null;
  camera.focus = focus > 0 ? focus : target ? target.distanceTo(new THREE.Vector3(...camera.position)) : 1;
  const ambientLights=(raw?.lights || []).filter((l:any)=>l.type==='ambient');
  const ambient = Number(value('ambient', 0.4));
  const sky = new THREE.Color(value('ambientColor', '#ffffff'));
  // three's hemisphere light lights a diffuse surface with albedo × color ×
  // intensity; a dome of that radiance does the same.
  const zenith = sky.clone().multiplyScalar(ambient);
  const ground = zenith.clone().multiplyScalar(0.35);
  const horizon = zenith.clone().lerp(ground, 0.5);
  const ambientFill=new THREE.Color(0,0,0);
  for(const light of ambientLights)ambientFill.add(new THREE.Color(channel(light,'light','color','#FFFFFF')).multiplyScalar(Number(channel(light,'light','intensity',1))));
  const catcher = !!env?.shadowCatcher && !bounds.isEmpty();
  const background = env?.background ? new THREE.Color(env.background) : catcher ? new THREE.Color(STUDIO_BACKDROP) : null;
  const display = background ? background.clone().convertLinearToSRGB() : null;
  return {
    camera,ambient:ambientFill.toArray() as [number,number,number],
    exposure: Number(value('exposure', 1)),
    backdrop: display ? [display.r, display.g, display.b] : [0, 0, 0],
    backdropOpaque: !!background,
    shadowCatcher: catcher,
    shadows: env?.shadows !== false,
    floorY: catcher ? bounds.min.y : 0,
    environment: { zenith: [zenith.r, zenith.g, zenith.b], horizon: [horizon.r, horizon.g, horizon.b], ground: [ground.r, ground.g, ground.b] },
  };
}


function backendFailure(job:Job,PM:any,error:unknown):void {
  if(job.disposed)return;
  if(job.backend.id==='webgpu'){job.failure=String(error);PM.invalidate?.('render');return;}
  job.backend.dispose();job.backend=new WebGpuStudioBackend();job.ready=false;job.sceneKey='';job.viewKey='';job.generation++;job.failure=null;
  state(PM).declined=`Native Studio: ${String(error)}; using WebGPU`;
  chain(job,job.backend.initialize().then(()=>{job.ready=true;PM.invalidate?.('render');},failure=>{job.failure=String(failure);PM.invalidate?.('render');}));
}
function chain(job:Job,promise:Promise<void>):void {
  const pending=promise.then(()=>{if(job.chain===pending)job.chain=null;});job.chain=pending;
}
function queue(job: Job, operation: () => void | Promise<void>, PM: any): void {
  const run=()=>job.disposed?undefined:operation();
  if(job.chain){chain(job,job.chain.then(run).catch(error=>backendFailure(job,PM,error)));return;}
  try{const result=run();if(result && typeof result.then==='function')chain(job,result.catch(error=>backendFailure(job,PM,error)));}
  catch(error){backendFailure(job,PM,error);}
}
function disposeJob(job: Job): void { job.disposed=true;job.generation++; for (const f of job.frames.values()) f.release?.(); job.frames.clear(); job.backend.dispose(); }
function schedule(PM: any): void {
  const s = state(PM); if (s.tick !== null || s.preview === 'fast') return;
  s.tick = requestAnimationFrame(() => {
    s.tick = null; let more = false;
    for (const j of s.jobs.values()) {
      if (PM.playing || j.failure || performance.now() - j.usedAt > 2000) continue;
      if (j.samples >= REFINE_SAMPLES) { j.refining = false; continue; }
      if (performance.now() - j.changedAt >= REFINE_DELAY_MS) {j.refining = true;if(!j.pending.size && j.samples>=j.refineGoal)j.refineGoal=Math.min(REFINE_SAMPLES,Math.max(2,j.refineGoal+4));}
      more = true;
    }
    if (more) { PM.invalidate?.('render'); schedule(PM); }
  }) as unknown as number;
}
function retainCapture(capture:Capture,key:string,frame:StudioFrame,backendId:string):void {
  const canvas=new OffscreenCanvas(frame.width,frame.height);
  canvas.getContext('2d')!.drawImage(frame.source,0,0);
  capture.frames.get(key)?.release?.();
  capture.frames.set(key,{...frame,backendId,source:canvas,version:`capture:${nextCaptureFrame++}`,release:()=>{canvas.width=canvas.height=0;}});
}
function requestFrame(PM: any, job: Job, request: StudioRenderRequest, key: string, capture:Capture|null=null,captureKey=''): StudioFrame | null {
  const cached = job.frames.get(key); if (cached) {job.completedGeneration=job.generation;state(PM).backend=job.backend.id;if(capture){capture.completed.add(captureKey);retainCapture(capture,captureKey,cached,job.backend.id);}return cached;}
  if (job.pending.has(key) || !job.ready) return null;
  const generation = job.generation;
  let asynchronous=false;
  const accept = (frame: StudioFrame) => {
    if (job.disposed || job.generation !== generation && (!capture || state(PM).capture!==capture)) { frame.release?.(); return; }
    if(capture && state(PM).capture===capture)capture.completed.add(captureKey);
    if(asynchronous && capture && state(PM).capture===capture){
      // Transport canvases may be reused for the next shutter sample. Snapshot
      // on the GPU before another scene/time patch can change their pixels.
      retainCapture(capture,captureKey,frame,job.backend.id);
    }
    // One current frame for each layer/tier; release superseded transport buffers.
    const prefix = `${request.layer ?? ''}:${request.tier}:`;
    for (const [oldKey, old] of job.frames) if (oldKey.startsWith(prefix) && oldKey !== key) { old.release?.(); job.frames.delete(oldKey); }
    job.frames.set(key, frame); job.completedGeneration=generation; job.samples = Math.max(job.samples, request.tier === 'refine' ? frame.samples : 0); state(PM).backend=job.backend.id;state(PM).frameSamples=frame.samples;state(PM).frames++;
  };
  try {
    const result = job.chain ? job.chain.then(() => job.backend.render(request)) : job.backend.render(request);
    if (result && typeof (result as Promise<StudioFrame>).then === 'function') {
      asynchronous=true;
      const pending = Promise.resolve(result).then(accept).catch(error => backendFailure(job,PM,error)).finally(() => { job.pending.delete(key); PM.invalidate?.('render'); });
      job.pending.set(key, pending);chain(job,pending);return null;
    }
    accept(result as StudioFrame); return result as StudioFrame;
  } catch (error) { backendFailure(job,PM,error); return null; }
}

const compositingLayers=new WeakMap<object,{revision:number;ids:Set<string>}>();
function separateCompositing(PM:any,runtime:any,time:number):Set<string>{
 const previous=compositingLayers.get(runtime);if(previous && previous.revision===runtime.revision)return previous.ids;
 const ids=new Set<string>();
 for(const [id,object] of runtime.objects as Map<string,THREE.Object3D>){
  let separate=(object.userData.layerOpacity ?? 1)<.999 || object.userData.layerOccludes===false;
  const layer=PM.L?.(id);
  if(layer && ((layer.masks || []).some((m:any)=>typeof m.on==='object'?PM.evP(layer,m.on,time,`m.${m.id}.on`)!==false:m.on!==false) || (layer.fx || []).some((fx:any)=>typeof fx.on==='object'?PM.evP(layer,fx.on,time,`${fx.id}.$enabled`)!==false:fx.on!==false)))separate=true;
  object.traverseVisible((o:any)=>{if(!o.isMesh || o.userData.splat || o.userData.sceneId!==id)return;for(const m of Array.isArray(o.material)?o.material:[o.material])if(m && (m.opacity<.999 || m.transparent && m.depthWrite===false))separate=true;});
  if(separate)ids.add(id);
 }
 compositingLayers.set(runtime,{revision:runtime.revision,ids});return ids;
}

/** Every model, extrusion and 3D plane receives Studio pixels from the same depth-set trace. */
export function studioLayer(PM: any, runtime: any, w: number, h: number, objectId: string, depthIds: Set<string>, bottom: boolean, camera: THREE.Camera, renderScreen?: (comp: string) => Uint8Array | null, time: number = PM.time): { canvas: CanvasImageSource; version: string } | null {
  if (!studioEnabled(PM)) return null;
  const s = state(PM), final = !!PM._studioFinal, still = !!PM._studioStill;
  const width = Math.max(2, Math.min(8192, Math.round(w))), height = Math.max(2, Math.min(8192, Math.round(h)));
  runtime.resizeCamera(width, height, camera);
  const splat=studioSplatLayer(runtime,camera,objectId,depthIds,width,height,bottom);
  if(splat){s.frames++;return {canvas:splat,version:`splat:${runtime.revision}:${width}:${height}:${camera.matrixWorld.elements}:${camera.projectionMatrix.elements}`};}
  const cam = cameraOf(camera, width, height); if (!cam) throw new Error('Unsupported Studio camera');
  const separate=separateCompositing(PM,runtime,time);
  if(final && objectId && combineOpaqueDepthSet(PM,depthIds,separate,time,runtime)) {
    if(!bottom) {
      if(!emptyCombinedLayer){emptyCombinedLayer=new OffscreenCanvas(2,2);emptyCombinedLayer.getContext('2d')!.clearRect(0,0,2,2);}
      const canvas=emptyCombinedLayer;
      return {canvas,version:'opaque-depth-set-member'};
    }
    objectId='';
  }
  const capture=final || still?s.capture:null;
  const captureKey=capture?`${PM.animVersion?.() ?? ''}:${runtime.studioId ?? (runtime.studioId=nextRuntime++)}:${[...depthIds].sort()}:${objectId}:${time}:${width}:${height}:${bottom}:${JSON.stringify(cam)}:${PM._studioShutterSamples || 1}`:'';
  if(capture && PM._studioPreparing){capture.requested.add(captureKey);if(runtime.materialsPending){capture.loading=true;return null;}}
  const prepared=capture?.frames.get(captureKey);
  if(prepared){s.backend=prepared.backendId ?? s.backend;s.frameSamples=prepared.samples;return {canvas:prepared.source,version:prepared.version};}
  // The established synchronous readback API still works without preparation.
  // Prepared exports/agent frames consume native capture images above; an
  // unprepared readback uses the same Studio shader synchronously on WebGPU.
  const synchronous=!!PM._studioSyncCapture;
  const hiddenPrimary=new Set([...separate].filter(id=>id!==objectId && depthIds.has(id)));
  const fork=[...depthIds].some(id=>separate.has(id));
  const geometrySetKey=`${runtime.studioId ?? (runtime.studioId = nextRuntime++)}:${[...depthIds].sort().join(',')}`;
  const setKey = `${geometrySetKey}${fork?`:layer:${objectId}`:''}${synchronous?':sync':''}`;
  for (const [k, j] of s.jobs) {
    // Depth sets partition a composition. An overlapping, different set is a
    // superseded scene layout, not another active view; release it immediately
    // so adding objects cannot occupy all native sessions with obsolete jobs.
    const overlap=j.runtime===runtime && [...depthIds].some(id=>j.depthIds.has(id));
    const obsolete=overlap && (j.separate!==fork || j.depthIds.size!==depthIds.size || [...depthIds].some(id=>!j.depthIds.has(id)));
    if (obsolete || performance.now() - j.usedAt > JOB_TTL_MS || j.factoryRevision !== studioBackendRevision()) { disposeJob(j); s.jobs.delete(k); }
  }
  let job = s.jobs.get(setKey);
  const screenKey = `${time}:${PM.animVersion?.() ?? ''}:${PM.preparedVideoVersion ?? ''}`;
  const sceneKey = `${runtime.revision}:${screenKey}`;
  if (!job || job.sceneKey !== sceneKey) {
    const env = { sky: new THREE.Color(), ground: new THREE.Color(), intensity: 0 };
    const extracted = extractScene(runtime.scene, id => depthIds.has(id), env, {hiddenPrimary, onReady: () => { s.revision++; PM.invalidate?.('render'); } });
    // Native, synchronous and per-layer views prepare the same geometry.
    // Share progress too, so a thumbnail cannot leave a stale Preparing entry
    // after the viewer has already received the worker's completed geometry.
    if (isPending(extracted)) { s.preparing.set(geometrySetKey, extracted); return null; }
    s.preparing.delete(geometrySetKey);
    attachStudioWorld(extracted,runtime.studioWorld,runtime.metersPerUnit);
    if (!studioBackendFactory() && extracted.tris.byteLength > studioMaxBindingBytes()) throw new Error('This scene exceeds the Studio GPU buffer budget');
    const screens: Job['screens'] = [];
    const maps = extracted.maps.map((m, layer) => { if ('screen' in m) { screens.push({ layer, comp: m.screen }); return renderScreen?.(m.screen) ?? new Uint8Array(MAP_SIZE * MAP_SIZE * 4); } return mapCanvas(m.source, m.flipY); });
    if (!job) {
      const backend = (!synchronous && studioBackendFactory()?.({ invalidate: () => PM.invalidate?.('render') })) || new WebGpuStudioBackend();
      job = { runtime, uid:nextJob++,depthIds:new Set(depthIds),separate:fork, sceneKey: '', viewKey: '', scene: extracted, maps, backend, ready: false, disposed:false, chain: null, frames: new Map(), pending: new Map(), screens, screenKey, changedAt: performance.now(), usedAt: performance.now(), refining: false, samples: 0, refineGoal:0, generation: 0, factoryRevision: studioBackendRevision(), completedGeneration: -1, failure: null };
      const created = job;
      if(backend.id==='webgpu' && studioGpuReady())created.ready=true;
      else chain(created,backend.initialize().then(() => { created.ready = true; PM.invalidate?.('render'); }, error => backendFailure(created,PM,error)));
      s.jobs.set(setKey, created);
    }
    job.sceneKey = sceneKey; job.scene = extracted; job.maps = maps; job.screens = screens; job.screenKey = screenKey;
    job.viewKey = ''; job.samples = 0; job.refineGoal=0; job.refining = false; job.changedAt = performance.now(); job.generation++;
    const view = viewOf(runtime, cam, extracted.bounds), current = job;
    queue(job, () => current.backend.setPreparedScene ? current.backend.setPreparedScene(extracted, maps, () => describePreparedScene(extracted, view, maps)) : current.backend.setScene(describePreparedScene(extracted, view, maps)), PM);
  }
  job.usedAt = performance.now();
  const view = viewOf(runtime, cam, job.scene.bounds), viewKey = JSON.stringify([view, width, height, time]);
  if (job.viewKey !== viewKey) {
    job.viewKey = viewKey; job.samples = 0; job.refineGoal=0; job.refining = false; job.changedAt = performance.now(); job.generation++;
    const current = job;
    queue(job, () => current.backend.setView ? current.backend.setView(view) : current.backend.setCamera(describeCamera(view).camera, describeCamera(view).options), PM);
    queue(job, () => current.backend.setTime(time), PM);
  }
  if (job.failure) throw new Error(job.failure);
  const tier = final || still ? 'final' : s.preview === 'quality' && performance.now() - job.changedAt >= REFINE_DELAY_MS && !PM.playing ? 'refine' : 'live';
  if(tier==='refine' && job.refineGoal===0)job.refineGoal=2;
  const samples = final ? Math.max(4, Math.ceil(Number(runtime.raw?.environment?.samples ?? STUDIO_FINAL_SAMPLES) / Math.max(1, PM._studioShutterSamples || 1))) : still ? STUDIO_STILL_SAMPLES : tier === 'refine' ? job.refineGoal : 1;
  // Native Live owns tracing resolution and MetalFX upscaling. Its requested
  // size is the final output size; do not halve it a second time in the page.
  const lowResolution=tier==='live' && job.backend.id==='webgpu';
  const rw = lowResolution ? Math.max(2, width >> 1) : width, rh = lowResolution ? Math.max(2, height >> 1) : height;
  const request: StudioRenderRequest = { tier, size: { width: rw, height: rh }, samples, layer: objectId || undefined, background: bottom, accumulate: tier === 'refine', denoise: studioOptions.denoise };
  const key = `${objectId}:${tier}:${bottom}:${job.sceneKey}:${viewKey}:${samples}:${rw}x${rh}`;
  const frame = requestFrame(PM, job, request, key,capture,captureKey);
  if (!final && !still) schedule(PM);
  if (frame) return { canvas: frame.source, version: `${job.uid}:${job.generation}:${frame.version}` };
  // Async renderers keep the last completed frame while a newer one is in flight.
  const previous = [...job.frames.entries()].reverse().find(([k]) => k.startsWith(`${objectId}:`))?.[1];
  return previous ? { canvas: previous.source, version: `${job.uid}:${job.generation}:${previous.version}` } : null;
}
let nextRuntime = 1,nextJob=1,nextCaptureFrame=1;
/** A capture owns async shutter images until the next frame's preparation. */
export function beginStudioCapture(PM:any):void {
  const s=state(PM);for(const frame of s.capture?.frames.values() || [])frame.release?.();s.capture={frames:new Map(),requested:new Set(),completed:new Set(),loading:false};
}
export function beginStudioCapturePass(PM:any):void {const capture=states.get(PM)?.capture;if(capture){capture.requested.clear();capture.loading=false;}}
export function pruneStudio(PM: any, keep: (setKey: string) => boolean): void {
  const s = states.get(PM); if (!s) return;
  for (const k of s.preparing.keys()) if (!keep(k)) s.preparing.delete(k);
  for (const [k, j] of s.jobs) if (!keep(k)) { disposeJob(j); s.jobs.delete(k); }
  if(!s.jobs.size){for(const frame of s.capture?.frames.values() || [])frame.release?.();s.capture=null;}
}
/** Wait for queued scene patches and frame transports, used before final readback. */
export async function studioIdle(PM?: any): Promise<void> {
  if (!PM) return;
  const s = states.get(PM); if (!s) return;
  await studioGeometryIdle();
  await Promise.all([...s.jobs.values()].flatMap(j => [j.chain, ...j.pending.values()].filter((p): p is Promise<void> => !!p)));
  const failure = [...s.jobs.values()].find(j => j.failure)?.failure; if (failure) throw new Error(failure);
}

export function studioPending(PM:any):boolean {
  const s=states.get(PM);if(s?.capture && PM._studioPreparing)return s.capture.loading || [...s.capture.requested].some(key=>!s.capture!.completed.has(key));
  return !!s && (s.preparing.size>0 || [...s.jobs.values()].some(j=>!j.ready || j.pending.size || j.completedGeneration!==j.generation || j.runtime.materialsPending));
}
