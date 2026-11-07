import { compositionById, compositionContains, materialOwner, materialLayerStatus, type MaterialMap } from './material-maps';
import { sourceTime } from '../../legacy/core/retiming';

export interface CompositionTextureFrame {pixels:Uint8Array<ArrayBuffer>;width:number;height:number;key:string;hasAlpha?:boolean}
const caches=new WeakMap<object,Map<string,CompositionTextureFrame>>();
const rendering=new WeakMap<object,Set<string>>();
export const MATERIAL_TEXTURE_MAX_SIZE=1024;
/** Source comps are sampled on their own frame grid, after the model's time mapping. */
export function materialCompositionSample(PM:any,layer:any,map:MaterialMap,time:number):{comp:any;time:number;width:number;height:number;key:string;sourceLayerId?:string}|null {
  if(typeof map==='string')return null;
  if('layerId' in map){
    const owner=materialOwner(PM.proj,layer.id);
    if(!owner || materialLayerStatus(PM.proj,layer.id,map.layerId)!=='ready')return null;
    const comp=compositionById(PM.proj,owner.id),source=comp.layers.find((l:any)=>l.id===map.layerId);
    const fps=Math.max(1,Number(comp.fps || 30)),frame=Math.floor(time*fps+1e-6),at=frame/fps;
    const bounds=PM.GL?.bounds?.(source,at),w=bounds?.w || source.d?.w?.v || source.d?.w || comp.w,h=bounds?.h || source.d?.h?.v || source.d?.h || comp.h;
    const scale=Math.min(1,MATERIAL_TEXTURE_MAX_SIZE/Math.max(w,h));
    const width=Math.max(2,Math.round(w*scale)),height=Math.max(2,Math.round(h*scale));
    return {comp,time:at,width,height,sourceLayerId:source.id,key:`layer:${owner.id}:${source.id}:${frame}:${width}x${height}:${PM.proj.revision || 0}:${PM._scene3dRevision || 0}`};
  }
  const comp=compositionById(PM.proj,map.compositionId);if(!comp)return null;
  const owner=PM._renderComposition3D || PM.curComp?.() || PM.proj,ownerId=owner.compId || owner.id;
  if(ownerId && compositionContains(PM.proj,map.compositionId,ownerId))return null;
  const fps=Math.max(1,Number(comp.fps || PM.proj.fps || 30));
  const frame=Math.max(0,Math.min(Math.max(0,Math.ceil(comp.dur*fps)-1),Math.floor(sourceTime(PM,layer,time)*fps+1e-6)));
  const scale=Math.min(1,MATERIAL_TEXTURE_MAX_SIZE/Math.max(comp.w,comp.h));
  const width=Math.max(2,Math.round(comp.w*scale)),height=Math.max(2,Math.round(comp.h*scale));
  const key=`${map.compositionId}:${frame}:${width}x${height}:${PM.proj.revision || 0}:${PM._scene3dRevision || 0}`;
  return {comp,time:frame/fps,width,height,key};
}
export function compositionTextureFrame(PM:any,layer:any,map:MaterialMap,time:number):CompositionTextureFrame|null {
  const sample=materialCompositionSample(PM,layer,map,time);if(!sample || !PM.GL?.renderMaterialComposition)return null;
  let cache=caches.get(PM);if(!cache){cache=new Map();caches.set(PM,cache);}
  const cached=cache.get(sample.key);if(cached)return cached;
  let active=rendering.get(PM);if(!active){active=new Set();rendering.set(PM,active);}
  const id=typeof map==='object'?JSON.stringify(map):'';
  if(active.has(id) || active.size>=8)return null;
  active.add(id);
  try{
    const pixels=PM.GL.renderMaterialComposition(sample.comp,sample.time,sample.width,sample.height,time,sample.sourceLayerId);
    if(!pixels)return null;
    let hasAlpha=false;for(let i=3;i<pixels.length;i+=4)if(pixels[i]!==255){hasAlpha=true;break;}
    const result={pixels,width:sample.width,height:sample.height,key:sample.key,hasAlpha};
    // At most 32 RGBA frames (128 MiB at the maximum resolution), per editor.
    if(cache.size>=32)cache.delete(cache.keys().next().value!);
    cache.set(sample.key,result);return result;
  }finally{active.delete(id);}
}
export function clearCompositionTextures(PM:any):void {caches.delete(PM);rendering.delete(PM);}
