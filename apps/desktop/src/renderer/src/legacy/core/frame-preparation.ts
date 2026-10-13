import { cancelPreviewVideoSeek } from './video-seek';
import { pauseVideoInstances } from './video-instances';
import { videoPlaybackTime } from '../../../../shared/video-timing';
import { sourceTime } from './retiming';

type FontReadiness = {
  fonts: FontFaceSet | null;
  generation: number;
  readyKey: unknown[] | null;
  pending: { key: unknown[]; promise: Promise<unknown> } | null;
  invalidate: () => void;
};
const fontReadiness = new WeakMap<object, FontReadiness>();
const fontEvents = ['loading', 'loadingdone', 'loadingerror'];
const sameFontKey = (first: unknown[] | null, second: unknown[]) => !!first && first.every((value, i) => Object.is(value, second[i]));

/** Chromium's resolved fonts.ready getter synchronously updates layout. Reuse
 * a completed wait until font loading or document content can change it. */
async function prepareFonts(PM: any, project: any): Promise<void> {
  let state = fontReadiness.get(PM);
  if (!state) {
    const created: FontReadiness = { fonts: null, generation: 0, readyKey: null, pending: null,
      invalidate: () => { created.generation++; created.readyKey = null; created.pending = null; } };
    state = created;
    fontReadiness.set(PM, state);
    for (const event of ['fonts', 'layers', 'project']) PM.bus?.on?.(event, state.invalidate);
    PM.Kernel?.events?.on?.('extensions:changed', state.invalidate);
    PM.Kernel?.events?.on?.('theme:changed', state.invalidate);
  }
  for (;;) {
    const fonts = document.fonts;
    if (!fonts) return;
    if (state.fonts !== fonts) {
      for (const event of fontEvents) state.fonts?.removeEventListener?.(event, state.invalidate);
      state.fonts = fonts; state.invalidate();
      for (const event of fontEvents) fonts.addEventListener?.(event, state.invalidate);
    }
    const key = () => [state!.generation, PM.proj, project, project.layers, project.layers?.length, PM.animVersion?.(), fonts.size];
    const captured = key();
    if (fonts.status !== 'loading' && sameFontKey(state.readyKey, captured)) return;
    const pending = sameFontKey(state.pending?.key ?? null, captured)
      ? state.pending! : { key: captured, promise: Promise.resolve(fonts.ready) };
    state.pending = pending;
    await pending.promise;
    // A new load, edit or FontFaceSet can arrive while the old promise settles.
    // Its result cannot stand in for the current document's readiness.
    if (document.fonts !== fonts || fonts.status === 'loading' || !sameFontKey(captured, key())) continue;
    state.readyKey = captured;
    if (state.pending === pending) state.pending = null;
    return;
  }
}

function seek(video:HTMLVideoElement,at:number):Promise<void>{return new Promise((resolve,reject)=>{video.pause();if(!video.seeking&&video.readyState>=2&&Math.abs(video.currentTime-at)<.0005)return resolve();const cleanup=()=>{clearTimeout(timer);video.removeEventListener('seeked',done);video.removeEventListener('error',fail);};const done=()=>{if(video.seeking||video.readyState<2||Math.abs(video.currentTime-at)>=.0005)return;cleanup();resolve();};const fail=()=>{cleanup();reject(new Error('Could not decode video frame'));};const timer=setTimeout(()=>{cleanup();reject(new Error('Video frame decode timed out'));},10000);video.addEventListener('seeked',done);video.addEventListener('error',fail,{once:true});video.currentTime=at;});}
/** Preserve each instance's decoded frame even when several layers share media. */
export async function prepareFrame(PM:any,time:number,project=PM.proj):Promise<void> {
  const requests:any[]=[];
  const visit=(comp:any,t:number,depth:number)=>{if(depth>8)throw new Error('Composition nesting is too deep');PM.scope.push(comp);try{for(const layer of comp.layers){if(!PM.active(layer,t))continue;if(layer.type==='video'){const asset=PM.assets.get(layer.d.asset);if(asset?.el)requests.push({layer,asset,time:t,at:videoPlaybackTime(asset,sourceTime(PM,layer,t),1/PM.proj.fps)});}else if(layer.type==='precomp'){const sub=comp.comps?.[layer.d.comp]||PM.proj.comps?.[layer.d.comp];if(sub)visit(sub,sourceTime(PM,layer,t),depth+1);}}}finally{PM.scope.pop();}};
  visit(project,time,0);const frames=new Map();
  for(const r of requests){pauseVideoInstances(r.asset);r.asset.preview?.el.pause();cancelPreviewVideoSeek(r.asset.el);await seek(r.asset.el,r.at);const cv=document.createElement('canvas');cv.width=r.asset.w||r.asset.el.videoWidth;cv.height=r.asset.h||r.asset.el.videoHeight;cv.getContext('2d')!.drawImage(r.asset.el,0,0,cv.width,cv.height);frames.set(r.layer.id+'@'+r.time,cv);}
  PM.preparedVideoFrames=frames;PM.preparedVideoVersion=(PM.preparedVideoVersion||0)+1;await prepareFonts(PM,project);
}
