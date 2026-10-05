import {beginBlenderCapture,prepareBlenderFrame} from '../../core/scene3d/rendering';
import { cancelPreviewVideoSeek } from './video-seek';
import { pauseVideoInstances } from './video-instances';
import { videoPlaybackTime } from '../../../../shared/video-timing';
import { sourceTime } from './retiming';
function seek(video:HTMLVideoElement,at:number):Promise<void>{return new Promise((resolve,reject)=>{video.pause();if(!video.seeking&&video.readyState>=2&&Math.abs(video.currentTime-at)<.0005)return resolve();const cleanup=()=>{clearTimeout(timer);video.removeEventListener('seeked',done);video.removeEventListener('error',fail);};const done=()=>{if(video.seeking||video.readyState<2||Math.abs(video.currentTime-at)>=.0005)return;cleanup();resolve();};const fail=()=>{cleanup();reject(new Error('Could not decode video frame'));};const timer=setTimeout(()=>{cleanup();reject(new Error('Video frame decode timed out'));},10000);video.addEventListener('seeked',done);video.addEventListener('error',fail,{once:true});video.currentTime=at;});}
/** Preserve each instance's decoded frame even when several layers share media. */
export async function prepareFrame(PM:any,time:number,project=PM.proj,options:{width?:number;height?:number;mblur?:boolean;mbSamples?:number;shutter?:number}={}):Promise<void> {
  const requests:any[]=[];
  const visit=(comp:any,t:number,depth:number)=>{if(depth>8)throw new Error('Composition nesting is too deep');PM.scope.push(comp);try{for(const layer of comp.layers){if(!PM.active(layer,t))continue;if(layer.type==='video'){const asset=PM.assets.get(layer.d.asset);if(asset?.el)requests.push({layer,asset,time:t,at:videoPlaybackTime(asset,sourceTime(PM,layer,t),1/PM.proj.fps)});}else if(layer.type==='precomp'){const sub=comp.comps?.[layer.d.comp]||PM.proj.comps?.[layer.d.comp];if(sub)visit(sub,sourceTime(PM,layer,t),depth+1);}}}finally{PM.scope.pop();}};
  visit(project,time,0);const frames=new Map();
  for(const r of requests){pauseVideoInstances(r.asset);r.asset.preview?.el.pause();cancelPreviewVideoSeek(r.asset.el);await seek(r.asset.el,r.at);const cv=document.createElement('canvas');cv.width=r.asset.w||r.asset.el.videoWidth;cv.height=r.asset.h||r.asset.el.videoHeight;cv.getContext('2d')!.drawImage(r.asset.el,0,0,cv.width,cv.height);frames.set(r.layer.id+'@'+r.time,cv);}
  options={...options,shutter:options.shutter||project.shutter||.5};
  beginBlenderCapture(PM);
  for(const item of blenderFrameRequests(PM,time,project,options)){
    await prepareBlenderFrame(PM,item.time,item.comp,{width:item.width,height:item.height,retain:true});
  }
  PM.preparedVideoFrames=frames;PM.preparedVideoVersion=(PM.preparedVideoVersion||0)+1;await document.fonts?.ready;
}

/** Mirror nested output sizes and shutter times before synchronous GPU compositing. */
export function blenderFrameRequests(PM:any,time:number,project=PM.proj,options:{width?:number;height?:number;mblur?:boolean;mbSamples?:number;shutter?:number}={}) {
  const frames=new Map<string,{comp:any;time:number;width:number;height:number}>(),visited=new Set<string>();
  const visit=(comp:any,t:number,width:number,height:number,depth:number)=>{
    if(depth>8)throw new Error('Composition nesting is too deep');
    const key=`${comp.compId||comp.id||'root'}:${t}:${width}x${height}`;
    if(visited.has(key))return;visited.add(key);
    if(visited.size>4096)throw new Error('Motion blur requires too many nested frames. Reduce shutter samples or composition nesting.');
    PM.scope.push(comp);
    try{
      const active=comp.layers.filter((layer:any)=>PM.active(layer,t));
      const blurred=options.mblur!==false&&active.some((layer:any)=>typeof layer.mblur==='object'?PM.evP(layer,layer.mblur,t,'l.mblur'):layer.mblur);
      const times=[t];
      if(blurred){const n=options.mbSamples||10,shutter=(options.shutter||.5)/comp.fps;for(let i=0;i<n;i++)times.push(t+((i+.5)/n-.5)*shutter);}
      for(const sampleTime of times){
        if(comp.render3d?.enabled&&active.some((layer:any)=>layer.d?.data?.object)){
          const frameKey=`${comp.compId||comp.id||'root'}:${sampleTime}:${width}x${height}`;frames.set(frameKey,{comp,time:sampleTime,width,height});
        }
        for(const layer of comp.layers){
          if(layer.type!=='precomp'||!PM.active(layer,sampleTime))continue;
          const sub=PM.compOf?.(layer)||comp.comps?.[layer.d.comp]||PM.proj.comps?.[layer.d.comp];
          if(!sub)continue;
          const w=Math.max(2,Math.round((layer.d.w||comp.w||width)*width/(comp.w||width))),h=Math.max(2,Math.round((layer.d.h||comp.h||height)*height/(comp.h||height)));
          visit(sub,sourceTime(PM,layer,sampleTime),w,h,depth+1);
        }
      }
    }finally{PM.scope.pop();}
  };
  visit(project,time,options.width||project.w,options.height||project.h,0);
  return [...frames.values()];
}
