import { compositionById, compositionContains, materialOwner, materialLayerStatus, type MaterialMap } from './material-maps';
import { sourceTime } from '../../legacy/core/retiming';
import { trackStaticEffects } from '../../legacy/core/static-effects';
import { materialStaticPlan } from './material-static';
import { materialTextureLimit, materialTextureSize, materialVideoTextureSize, type MaterialTextureSize } from './material-resolution';

export interface CompositionTextureFrame {pixels:Uint8Array<ArrayBuffer>;surface?:HTMLCanvasElement;width:number;height:number;key:string;hasAlpha?:boolean}
type StaticPlan = {source:ReturnType<typeof materialStaticPlan>['plan'];images:any[];sources:unknown[];version:number;renderStamp:unknown};
type TextureCache = {frames:Map<string,CompositionTextureFrame>;plans:Map<string,StaticPlan>;versions:Map<string,{signature:string;version:number;renderStamp?:unknown}>;
  effects:Map<string,ReturnType<typeof trackStaticEffects>>;nextVersion:number;bytes:number;stamp:unknown[];editStamp:unknown[];dispose:()=>void};
const caches=new WeakMap<object,TextureCache>();
const rendering=new WeakMap<object,Set<string>>();
const frameContents=new WeakMap<CompositionTextureFrame,string>();
const MATERIAL_CACHE_BYTES=128*1024*1024;
const same=(a:unknown[],b:unknown[])=>a.length===b.length && a.every((value,index)=>Object.is(value,b[index]));
function textureCache(PM:any):TextureCache {
  let cache=caches.get(PM);
  if(!cache){
    const frames=new Map<string,CompositionTextureFrame>(),plans=new Map<string,StaticPlan>();
    const clear=()=>{frames.clear();plans.clear();if(cache)cache.bytes=0;};
    const offs=[PM.bus?.on?.('project',()=>plans.clear()),...['assets','fonts','quality'].map(event=>PM.bus?.on?.(event,clear)),
      ...['extensions:changed','theme:changed'].map(event=>PM.Kernel?.events?.on?.(event,clear))];
    const effects=new Map<string,ReturnType<typeof trackStaticEffects>>();
    cache={frames,plans,versions:new Map(),effects,nextVersion:0,bytes:0,stamp:[],editStamp:[],dispose:()=>{for(const tracker of effects.values())tracker.dispose();for(const off of offs)if(typeof off==='function')off();else off?.dispose?.();}};
    caches.set(PM,cache);
  }
  const fonts=typeof document==='undefined'?undefined:document.fonts;
  const stamp=[PM.proj,PM.GL?.gl,PM.proj.colorDepth,fonts,fonts?.status,fonts?.size];
  if(!same(cache.stamp,stamp)){cache.frames.clear();cache.bytes=0;cache.plans.clear();cache.versions.clear();cache.stamp=stamp;}
  const editStamp=[PM.proj.revision,PM._scene3dRevision,PM.animVersion?.()];
  if(!same(cache.editStamp,editStamp)){cache.plans.clear();cache.editStamp=editStamp;}
  return cache;
}
function sourceFrameKey(PM:any,cache:TextureCache,sample:NonNullable<ReturnType<typeof materialCompositionSample>>):string {
  const id=`${sample.comp.compId || sample.comp.id}:${sample.sourceLayerId || ''}`;
  let plan=cache.plans.get(id);
  const imageSources=(images:any[])=>images.flatMap(layer=>{
    const asset=PM.assets?.get?.(layer.d?.asset),el=asset?.el;
    return [asset,el,el?.width,el?.height,asset?.w,asset?.h,asset?.url,asset?.format,asset?.imageSequence];
  });
  if(!plan){
    const {plan:source,images}=materialStaticPlan(PM,sample.comp,sample.sourceLayerId);
    // Even an unavailable image must be observed so decoding can enable reuse.
    let version=0;
    let renderStamp:unknown;
    if(source){
      // Recheck source content after edits, but keep its pixels when only the
      // receiving model/camera changed. Never stringify the scene per frame.
      const signature=JSON.stringify(source.content);
      const previous=cache.versions.get(id);
      version=previous?.signature===signature?previous.version:++cache.nextVersion;
      renderStamp=previous?.renderStamp;
      cache.versions.delete(id);cache.versions.set(id,{signature,version,renderStamp});
      if(cache.versions.size>64)cache.versions.delete(cache.versions.keys().next().value!);
    }
    plan={source,images,sources:imageSources(images),version,renderStamp};cache.plans.set(id,plan);
  }
  const sources=imageSources(plan.images);
  if(!same(plan.sources,sources)){cache.frames.clear();cache.bytes=0;cache.plans.delete(id);return sourceFrameKey(PM,cache,sample);}
  if(plan.source && (plan.source.effects.length || plan.source.programs.length)){
    let tracker=cache.effects.get(id);
    if(!tracker){
      tracker=trackStaticEffects(PM);cache.effects.set(id,tracker);
      if(cache.effects.size>64){const oldest=cache.effects.keys().next().value!;cache.effects.get(oldest)!.dispose();cache.effects.delete(oldest);}
    }
    const stamp=tracker.capture(plan.source.effects,plan.source.programs);
    if(stamp!==plan.renderStamp){
      if(plan.renderStamp!==undefined)plan.version=++cache.nextVersion;
      plan.renderStamp=stamp;
      const version=cache.versions.get(id);
      if(version){version.version=plan.version;version.renderStamp=stamp;}
    }
    if(!stamp)return sample.contentKey;
  }
  const timeKey=plan.source?.key(sample.time);
  return timeKey==null?sample.contentKey:`${id}:${plan.version}:${timeKey}`;
}
/** Source comps are sampled on their own frame grid, after the model's time mapping. */
export function materialCompositionSample(PM:any,layer:any,map:MaterialMap,time:number,minimum?:MaterialTextureSize):{comp:any;time:number;width:number;height:number;key:string;contentKey:string;sourceLayerId?:string}|null {
  if(typeof map==='string')return null;
  if('layerId' in map){
    const owner=materialOwner(PM.proj,layer.id);
    if(!owner || materialLayerStatus(PM.proj,layer.id,map.layerId)!=='ready')return null;
    const comp=compositionById(PM.proj,owner.id),source=comp.layers.find((l:any)=>l.id===map.layerId);
    const fps=Math.max(1,Number(comp.fps || 30)),frame=Math.floor(time*fps+1e-6),at=frame/fps;
    const bounds=PM.GL?.materialSourceBounds?.(source,at) ?? PM.GL?.bounds?.(source,at),w=bounds?.w || source.d?.w?.v || source.d?.w || comp.w,h=bounds?.h || source.d?.h?.v || source.d?.h || comp.h;
    const asset=source.type==='image'?PM.assets?.get?.(source.d?.asset):null,el=asset?.el || asset?.bitmap;
    const imageWidth=el?.naturalWidth || el?.width || asset?.w || w,imageHeight=el?.naturalHeight || el?.height || asset?.h || h;
    const nativeScale=asset?Math.max(1,imageWidth/w,imageHeight/h):1;
    const size=source.type==='video'?materialVideoTextureSize:materialTextureSize;
    const {width,height}=size(w*nativeScale,h*nativeScale,minimum,materialTextureLimit(PM));
    const contentKey=`layer:${owner.id}:${source.id}:${frame}:${PM.proj.revision || 0}:${PM._scene3dRevision || 0}`;
    return {comp,time:at,width,height,sourceLayerId:source.id,contentKey,key:`${contentKey}:${width}x${height}`};
  }
  const comp=compositionById(PM.proj,map.compositionId);if(!comp)return null;
  const owner=PM._renderComposition3D || PM.curComp?.() || PM.proj,ownerId=owner.compId || owner.id;
  if(ownerId && compositionContains(PM.proj,map.compositionId,ownerId))return null;
  const fps=Math.max(1,Number(comp.fps || PM.proj.fps || 30));
  const frame=Math.max(0,Math.min(Math.max(0,Math.ceil(comp.dur*fps)-1),Math.floor(sourceTime(PM,layer,time)*fps+1e-6)));
  const {width,height}=materialTextureSize(comp.w,comp.h,minimum,materialTextureLimit(PM));
  const contentKey=`${map.compositionId}:${frame}:${PM.proj.revision || 0}:${PM._scene3dRevision || 0}`;
  return {comp,time:frame/fps,width,height,contentKey,key:`${contentKey}:${width}x${height}`};
}
export function compositionTextureFrame(PM:any,layer:any,map:MaterialMap,time:number,onLoading?:()=>void,minimum?:MaterialTextureSize):CompositionTextureFrame|null {
  const sample=materialCompositionSample(PM,layer,map,time,minimum);if(!sample || !PM.GL?.renderMaterialComposition)return null;
  if(sample.sourceLayerId && PM.GL.renderMaterialVideo){
    const frame=PM.GL.renderMaterialVideo(sample.comp,sample.time,sample.width,sample.height,time,sample.sourceLayerId);
    if(frame==='loading'){onLoading?.();return null;}
    if(frame)return frame;
  }
  const cache=textureCache(PM),contentKey=sourceFrameKey(PM,cache,sample),key=`${contentKey}:${sample.width}x${sample.height}`;
  const cached=cache.frames.get(key);if(cached){cache.frames.delete(key);cache.frames.set(key,cached);return cached;}
  // Different material slots/UV islands can request different densities of
  // the same frame. An already sharper capture serves the smaller request.
  for(const [previousKey,frame] of cache.frames)if(frameContents.get(frame)===contentKey && frame.width>=sample.width && frame.height>=sample.height){
    cache.frames.delete(previousKey);cache.frames.set(previousKey,frame);return frame;
  }
  let active=rendering.get(PM);if(!active){active=new Set();rendering.set(PM,active);}
  const id=typeof map==='object'?JSON.stringify(map):'';
  if(active.has(id) || active.size>=8)return null;
  active.add(id);
  try{
    const pixels=PM.GL.renderMaterialComposition(sample.comp,sample.time,sample.width,sample.height,time,sample.sourceLayerId);
    if(!pixels){onLoading?.();return null;}
    let hasAlpha=false;for(let i=3;i<pixels.length;i+=4)if(pixels[i]!==255){hasAlpha=true;break;}
    const result={pixels,width:sample.width,height:sample.height,key,hasAlpha};
    frameContents.set(result,contentKey);
    // Budget bytes, not just frame count: a native 4K texture is much larger
    // than a thumbnail. Keep a single oversized frame to avoid re-capturing it
    // on every paint; it is replaced before retaining any additional frames.
    const budget=Math.min(MATERIAL_CACHE_BYTES,PM.Memory?.budget?.('preview') ?? MATERIAL_CACHE_BYTES);
    while(cache.frames.size && (cache.frames.size>=32 || cache.bytes+pixels.byteLength>budget)){
      const oldest=cache.frames.keys().next().value!;cache.bytes-=cache.frames.get(oldest)!.pixels.byteLength;cache.frames.delete(oldest);
    }
    // A pending font can change otherwise static text without a project edit.
    if(typeof document==='undefined' || document.fonts?.status!=='loading'){cache.frames.set(key,result);cache.bytes+=pixels.byteLength;}
    return result;
  }finally{active.delete(id);}
}
export function clearCompositionTextures(PM:any):void {caches.get(PM)?.dispose();caches.delete(PM);rendering.delete(PM);}
