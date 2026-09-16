import { cancelPreviewVideoSeek } from './video-seek';
import { sequencePlaybackTime } from '../../../../shared/image-sequence';
import { sourceTime } from './retiming';
function seek(video:HTMLVideoElement,at:number):Promise<void>{return new Promise((resolve,reject)=>{video.pause();if(!video.seeking&&video.readyState>=2&&Math.abs(video.currentTime-at)<.0005)return resolve();const cleanup=()=>{clearTimeout(timer);video.removeEventListener('seeked',done);video.removeEventListener('error',fail);};const done=()=>{cleanup();resolve();};const fail=()=>{cleanup();reject(new Error('Could not decode video frame'));};const timer=setTimeout(()=>{cleanup();reject(new Error('Video frame decode timed out'));},10000);video.addEventListener('seeked',done,{once:true});video.addEventListener('error',fail,{once:true});video.currentTime=at;});}
/** Preserve each instance's decoded frame even when several layers share media. */
export async function prepareFrame(PM:any,time:number,project=PM.proj):Promise<void> {
  const requests:any[]=[];
  const visit=(comp:any,t:number,depth:number)=>{if(depth>8)throw new Error('Composition nesting is too deep');PM.scope.push(comp);try{for(const layer of comp.layers){if(!PM.active(layer,t))continue;if(layer.type==='video'){const asset=PM.assets.get(layer.d.asset);if(asset?.el)requests.push({layer,asset,time:t,at:sequencePlaybackTime(asset,sourceTime(PM,layer,t))??Math.max(0,Math.min(Math.max(0,asset.dur-1/PM.proj.fps),sourceTime(PM,layer,t)))});}else if(layer.type==='precomp'){const sub=comp.comps?.[layer.d.comp]||PM.proj.comps?.[layer.d.comp];if(sub)visit(sub,sourceTime(PM,layer,t),depth+1);}}}finally{PM.scope.pop();}};
  visit(project,time,0);const frames=new Map();
  for(const r of requests){r.asset.preview?.el.pause();cancelPreviewVideoSeek(r.asset.el);await seek(r.asset.el,r.at);const cv=document.createElement('canvas');cv.width=r.asset.w||r.asset.el.videoWidth;cv.height=r.asset.h||r.asset.el.videoHeight;cv.getContext('2d')!.drawImage(r.asset.el,0,0,cv.width,cv.height);frames.set(r.layer.id+'@'+r.time,cv);}
  PM.preparedVideoFrames=frames;PM.preparedVideoVersion=(PM.preparedVideoVersion||0)+1;await document.fonts?.ready;
}
