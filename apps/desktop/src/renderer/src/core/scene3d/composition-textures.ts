import { compositionById, compositionContains, materialOwner, materialLayerStatus, materialSourceLayers, type MaterialMap } from './material-maps';
import { sourceTime } from '../../legacy/core/retiming';
import { staticExportPlan } from '../../legacy/core/static-export';
import { materialTextureLimit, materialTextureSize, type MaterialTextureSize } from './material-resolution';

export interface CompositionTextureFrame {pixels:Uint8Array<ArrayBuffer>;width:number;height:number;key:string;hasAlpha?:boolean}
type StaticPlan = {key:ReturnType<typeof staticExportPlan>;images:any[];sources:unknown[];version:number};
type TextureCache = {frames:Map<string,CompositionTextureFrame>;plans:Map<string,StaticPlan>;versions:Map<string,{signature:string;version:number}>;
  nextVersion:number;bytes:number;stamp:unknown[];editStamp:unknown[];dispose:()=>void};
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
    cache={frames,plans,versions:new Map(),nextVersion:0,bytes:0,stamp:[],editStamp:[],dispose:()=>{for(const off of offs)if(typeof off==='function')off();else off?.dispose?.();}};
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
    const layers=sample.sourceLayerId?materialSourceLayers(sample.comp,sample.sourceLayerId):sample.comp.layers;
    if(sample.sourceLayerId && layers.some((layer:any)=>layer.type==='group')){
      // External parent rigs affect child placement too. Include their channels
      // in the static plan so an animated rig cannot freeze a group texture.
      const byId=new Map<string,any>(sample.comp.layers.map((layer:any)=>[layer.id,layer])),seen=new Set(layers.map((layer:any)=>layer.id));
      for(let i=0;i<layers.length;i++)for(const id of [layers[i].parent,layers[i].group])if(id&&!seen.has(id)&&byId.has(id)){seen.add(id);layers.push(byId.get(id));}
    }
    const images=layers.filter((layer:any)=>layer.type==='image');
    // Only immutable, decoded bitmaps may be retained across time. Mutable
    // canvases, video, expressions, effects and unknown renderers stay exact.
    const key=staticExportPlan({...sample.comp,layers},false,layer=>{
      const asset=PM.assets?.get?.(layer.d?.asset),el=asset?.el;
      return asset?.kind==='image' && !asset.imageSequence && asset.format!=='svg'
        && typeof ImageBitmap!=='undefined' && el instanceof ImageBitmap && el.width>0 && el.height>0;
    });
    let version=0;
    if(key){
      // Recheck source content after edits, but keep its pixels when only the
      // receiving model/camera changed. Never stringify the scene per frame.
      const comp=sample.comp,signature=JSON.stringify([comp.w,comp.h,comp.fps,comp.dur,comp.bg,comp.backgroundFill,layers]);
      const previous=cache.versions.get(id);
      version=previous?.signature===signature?previous.version:++cache.nextVersion;
      cache.versions.delete(id);cache.versions.set(id,{signature,version});
      if(cache.versions.size>64)cache.versions.delete(cache.versions.keys().next().value!);
    }
    plan={key,images,sources:imageSources(images),version};cache.plans.set(id,plan);
  }
  const sources=imageSources(plan.images);
  if(!same(plan.sources,sources)){cache.frames.clear();cache.bytes=0;cache.plans.delete(id);return sourceFrameKey(PM,cache,sample);}
  const timeKey=plan.key?.(sample.time);
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
    const {width,height}=materialTextureSize(w*nativeScale,h*nativeScale,minimum,materialTextureLimit(PM));
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
