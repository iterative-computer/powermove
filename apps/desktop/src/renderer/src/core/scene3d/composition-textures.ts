import { compositionById, compositionContains, type MaterialMap } from './material-maps';
import { sourceTime } from '../../legacy/core/retiming';

export interface CompositionTextureFrame {pixels:Uint8Array<ArrayBuffer>;width:number;height:number;key:string}
const caches=new WeakMap<object,Map<string,CompositionTextureFrame>>();
const rendering=new WeakMap<object,Set<string>>();
export const MATERIAL_TEXTURE_MAX_SIZE=1024;
/** Source comps are sampled on their own frame grid, after the model's time mapping. */
export function materialCompositionSample(PM:any,layer:any,map:MaterialMap,time:number):{comp:any;time:number;width:number;height:number;key:string}|null {
  if(typeof map==='string')return null;
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
  const id=typeof map==='object'?map.compositionId:'';
  if(active.has(id) || active.size>=8)return null;
  active.add(id);
  try{
    const pixels=PM.GL.renderMaterialComposition(sample.comp,sample.time,sample.width,sample.height,time);
    if(!pixels)return null;
    const result={pixels,width:sample.width,height:sample.height,key:sample.key};
    // At most 32 RGBA frames (128 MiB at the maximum resolution), per editor.
    if(cache.size>=32)cache.delete(cache.keys().next().value!);
    cache.set(sample.key,result);return result;
  }finally{active.delete(id);}
}
export function clearCompositionTextures(PM:any):void {caches.delete(PM);rendering.delete(PM);}
