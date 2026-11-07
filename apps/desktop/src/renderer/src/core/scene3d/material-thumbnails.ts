import { viewerService } from '../../legacy/core/services';
import { canAssignMaterialMap, materialOwner, type MaterialMap } from './material-maps';
import { materialCompositionSample } from './composition-textures';

export interface MaterialThumbnail {url:string;width:number;height:number}
export interface MaterialThumbnailOptions {size?:number;pixelRatio?:number;time?:number}
const MAX_ENTRIES=64;
const MAX_PENDING=64;
const bounded=(n:number|undefined,fallback:number,min:number,max:number)=>Math.max(min,Math.min(max,Number.isFinite(n)?n!:fallback));

/** Picker-only cache. No thumbnail work occurs while listing sources or during
 * a viewer render. One small capture per idle turn; null leaves the UI placeholder. */
export function createMaterialThumbnails(PM:any) {
  const cache=new Map<string,MaterialThumbnail>();
  const pending=new Map<string,Promise<MaterialThumbnail|null>>();
  const identities=new WeakMap<object,number>();let nextIdentity=0;
  const identity=(value:any)=>{
    if(!value || typeof value!=='object')return 0;
    if(!identities.has(value))identities.set(value,++nextIdentity);
    return identities.get(value);
  };
  let generation=0,scheduled=false,disposed=false;
  const queue:{run:()=>void;cancel:()=>void}[]=[];
  const clear=()=>{generation++;cache.clear();pending.clear();for(const job of queue.splice(0))job.cancel();};
  const offs=['project','layers','comps','history:project-patch','assets'].map(event=>PM.bus?.on?.(event,clear));
  const schedule=()=>{
    if(scheduled || !queue.length || disposed)return;
    scheduled=true;
    const run=()=>{scheduled=false;queue.shift()?.run();schedule();};
    if(typeof globalThis.requestIdleCallback==='function')globalThis.requestIdleCallback(run,{timeout:250});
    else setTimeout(run,16);
  };
  const get=(receiverId:string,source:MaterialMap,options:MaterialThumbnailOptions={}):Promise<MaterialThumbnail|null>=>{
    if(disposed || !Number.isFinite(options.time ?? PM.time ?? 0)
        || !canAssignMaterialMap(PM,receiverId,null,'color',source).ok)return Promise.resolve(null);
    const receiver=materialOwner(PM.proj,receiverId)?.layers.find((l:any)=>l.id===receiverId);
    const time=options.time ?? PM.time ?? 0;
    const size=Math.round(bounded(options.size,64,16,64)*bounded(options.pixelRatio,globalThis.devicePixelRatio || 1,1,2));
    const version=[identity(PM.proj),PM.proj.revision,PM._scene3dRevision,PM.animVersion?.()];
    const asset=typeof source==='string'?PM.assets?.get?.(source):null;
    const key=JSON.stringify([receiverId,source,time,size,version,identity(asset?.el)]);
    const hit=cache.get(key);
    if(hit){cache.delete(key);cache.set(key,hit);return Promise.resolve(hit);}
    if(pending.has(key))return pending.get(key)!;
    const epoch=generation;
    let finish!:(value:MaterialThumbnail|null)=>void;
    const promise=new Promise<MaterialThumbnail|null>(resolve=>{finish=resolve;});pending.set(key,promise);
    const valid=()=>!disposed && generation===epoch && identity(PM.proj)===version[0]
      && PM.proj.revision===version[1] && PM._scene3dRevision===version[2] && PM.animVersion?.()===version[3];
    const complete=(result:MaterialThumbnail|null)=>{
      if(pending.get(key)===promise)pending.delete(key);
      if(result && valid()){
        cache.set(key,result);
        while(cache.size>MAX_ENTRIES)cache.delete(cache.keys().next().value!);
      }
      finish(valid()?result:null);
    };
    const job={cancel:()=>complete(null),run:()=>{
      // Idle captures must not steal time from playback, scrubbing or export.
      if(!valid() || PM.playing || PM.Export?.busy || PM.agentFrameCapture || PM.Preview?.preparing || PM.Preview?.active
          || PM.interactionActive?.() || viewerService(PM)?.isNavigating?.()){complete(null);return;}
      try{
        const canvas=document.createElement('canvas'),context=canvas.getContext('2d');
        if(!context){complete(null);return;}
        if(typeof source==='string'){
          const el=asset?.kind==='image' && asset.el;
          const w=el?.naturalWidth || el?.width || asset?.w,h=el?.naturalHeight || el?.height || asset?.h;
          if(!el || !(w>0 && h>0)){complete(null);return;}
          const scale=size/Math.max(w,h);canvas.width=Math.max(1,Math.round(w*scale));canvas.height=Math.max(1,Math.round(h*scale));
          context.drawImage(el,0,0,canvas.width,canvas.height);
        }else{
          const sample=materialCompositionSample(PM,receiver,source,time);
          if(!sample || !PM.GL?.renderMaterialComposition){complete(null);return;}
          const scale=size/Math.max(sample.width,sample.height);
          canvas.width=Math.max(1,Math.round(sample.width*scale));canvas.height=Math.max(1,Math.round(sample.height*scale));
          const pixels=PM.GL.renderMaterialComposition(sample.comp,sample.time,canvas.width,canvas.height,PM.time,sample.sourceLayerId);
          if(!pixels || pixels.length!==canvas.width*canvas.height*4){complete(null);return;}
          context.putImageData(new ImageData(new Uint8ClampedArray(pixels),canvas.width,canvas.height),0,0);
        }
        complete({url:canvas.toDataURL('image/png'),width:canvas.width,height:canvas.height});
      }catch{complete(null);}
      finally{PM.beginEval?.(PM.time);}
    }};
    while(queue.length>=MAX_PENDING)queue.shift()!.cancel();
    queue.push(job);schedule();return promise;
  };
  return {get,clear,dispose(){disposed=true;clear();for(const off of offs)if(typeof off==='function')off();}};
}
