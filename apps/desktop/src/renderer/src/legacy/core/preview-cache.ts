import { prepareFrame } from './frame-preparation';
const LIMIT=256*1024*1024;
export function installPreviewCache(PM:any) {
  let generation=0,bytes=0;const frames=new Map<number,ImageBitmap>();let signature='',preparing=false,active=false,canvas:HTMLCanvasElement|null=null,raf=0;
  let cachedTransport:any=null;
  const key=()=>[PM.proj.id,PM.animVersion?.()||0,PM.GL.canvas?.width||0,PM.GL.canvas?.height||0,PM.previewFps||PM.proj.fps,PM.proj.work?.join(',')].join('|');
  const stopAudio=()=>{
    if(cachedTransport){const transport=cachedTransport;cachedTransport=null;try{transport.pause();}finally{transport.setLoop(PM.loop?(PM.proj.work?.[1]>PM.proj.work?.[0]?PM.proj.work:[0,PM.proj.dur]):null);} }
    else PM.Audio?.pause();
  };
  // Cache invalidation also happens during ordinary playback (for example,
  // adaptive quality changes). Only an active cached preview owns its audio.
  const clear=()=>{generation++;if(active)stopAudio();active=false;cancelAnimationFrame(raf);canvas?.remove();canvas=null;for(const f of frames.values())f.close();frames.clear();bytes=0;PM.bus.emit('preview');};
  const stop=()=>{
    const wasActive=active;
    if(wasActive)stopAudio();
    active=false;cancelAnimationFrame(raf);canvas?.remove();canvas=null;
    // Match normal playback: editing resumes on the project frame displayed
    // beneath the continuous transport position, never a rounded future frame.
    if(wasActive)PM.setTime(Math.floor(PM.time*PM.proj.fps+1e-7)/PM.proj.fps,{raw:true});
    PM.bus.emit('preview');PM.invalidate();
  };
  const play=()=>{if(!frames.size)return;if(signature!==key()){clear();return;}PM.pause();active=true;const fps=PM.previewFps||PM.proj.fps,start=PM.proj.work?.[0]||0;canvas=document.createElement('canvas');canvas.width=PM.GL.canvas.width;canvas.height=PM.GL.canvas.height;canvas.style.cssText='position:absolute;inset:0;width:100%;height:100%;pointer-events:none;z-index:2';PM.GL.canvas.parentElement?.appendChild(canvas);const ctx=canvas.getContext('2d')!;cachedTransport=PM.EngineConnection?.transport?.active?.()?PM.EngineConnection.transport:null;if(cachedTransport){cachedTransport.setLoop([start,start+frames.size/fps]);cachedTransport.play(start);}else PM.Audio.start(start);const first=performance.now();let lastCycle=0,lastTick=first,lastPicture=-1,lastNativeTime=start;
    // RAF's shared timestamp can precede setup performed within that frame.
    const tick=(now:number)=>{
      if(!active)return;if(signature!==key()){clear();return;}
      // Register the next clock update before listeners queue their paints, so
      // the next display refresh paints its current position rather than the
      // previous refresh's time. Bitmap selection remains frame accurate.
      raf=requestAnimationFrame(tick);
      const elapsed=Math.max(0,now-first)/1000,span=frames.size/fps;
      const cycle=Math.floor(elapsed/span),local=elapsed-cycle*span;
      const position=cachedTransport?.now(now);
      if(position?.playing&&position.time>=start&&position.time<start+span)lastNativeTime=position.time;
      const time=cachedTransport?lastNativeTime:start+local;
      const i=Math.min(frames.size-1,Math.floor((time-start)*fps+1e-7));
      if(!cachedTransport&&(cycle!==lastCycle||now-lastTick>250))PM.Audio.seek(time);
      lastCycle=cycle;lastTick=now;
      if(i!==lastPicture){ctx.drawImage(frames.get(i)!,0,0);lastPicture=i;}
      // Legacy audio still needs live ticks for upcoming cuts and voices.
      // Native playback uses the transport's audio clock for both sound and picture.
      PM.time=time;if(!cachedTransport)PM.Audio.tick(time);PM.bus.emit('time',time);PM.bus.emit('overlay');PM.invalidate('timeline');
    };raf=requestAnimationFrame(tick);PM.bus.emit('preview');};
  PM.Preview={clear,stop,play,get active(){return active;},get preparing(){return preparing;},get count(){return frames.size;},get bytes(){return bytes;},cancel:()=>{generation++;},async cache(){if(preparing)return;clear();PM.GL.previewFrames?.clear();preparing=true;PM.pause();const own=generation,old=PM.time,w=PM.GL.canvas.width,h=PM.GL.canvas.height,fps=PM.previewFps||PM.proj.fps,[start,end]=PM.proj.work||[0,PM.proj.dur],count=Math.max(1,Math.ceil((end-start)*fps));signature=key();
    try{const limit=()=>PM.Memory?.budget?.('preview')??LIMIT;const required=count*w*h*4;if(required>limit())throw new Error(limit()===0?'Preview memory is off. Enable it in Settings → General.':'This preview exceeds your memory limit. Shorten the work area, lower preview resolution, or raise the limit in Settings → General.');for(let i=0;i<count;i++){if(own!==generation||signature!==key()||required>limit())break;const time=start+i/fps;let source:HTMLCanvasElement;if(PM.EngineConnection){source=await PM.EngineConnection.render.renderFrameTo(time,w,h,{mblur:true,mbSamples:6});}else{await prepareFrame(PM,time);PM.GL.render(time,{mblur:true,mbSamples:6});source=PM.GL.canvas;}if(own!==generation||signature!==key()||required>limit())break;const bitmap=await createImageBitmap(source);if(own!==generation||signature!==key()||required>limit()){bitmap.close();break;}frames.set(i,bitmap);bytes+=w*h*4;PM.bus.emit('preview');await new Promise(r=>setTimeout(r,0));}if(frames.size===count&&own===generation)play();else clear();}
    catch(e){clear();PM.toast((e as Error).message,6000);}finally{preparing=false;if(!active)PM.time=old;PM.bus.emit('preview');PM.invalidate();}
  }};
  PM.bus.on('project',clear);PM.bus.on('layers',()=>{if(signature!==key())clear();});PM.bus.on('quality',clear);
}
