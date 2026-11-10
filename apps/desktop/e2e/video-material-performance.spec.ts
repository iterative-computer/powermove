import {test,expect,repoRoot} from './helpers/app';
import {screenGlb} from '../src/renderer/src/core/scene3d/__fixtures__/screen-glb';
import {execFileSync} from 'node:child_process';
import {readFile} from 'node:fs/promises';
import path from 'node:path';

test('HD video materials stream through GPU surfaces without material readbacks and retain exact capture',async({session},info)=>{
  test.setTimeout(120_000);await session.openEditor();
  const movie=path.join(session.userData,'material.mp4');
  execFileSync(path.join(repoRoot,'node_modules/ffmpeg-static/ffmpeg'),[
    '-v','error','-f','lavfi','-i','testsrc2=size=1920x1080:rate=30','-t','4',
    '-c:v','libx264','-preset','ultrafast','-crf','18','-g','120','-pix_fmt','yuv420p',movie,
  ]);
  const result=await session.page.evaluate(async({movie,model})=>{
    const PM=(window as any).PM;PM.pause();PM.AutomaticPreview?.cancel?.();PM.perf.auto=false;PM.quality=1;
    PM.replaceProject(PM.mkProject({w:640,h:360,fps:30,dur:4}));
    const asset=await PM.assets.add(new File([new Uint8Array(movie)],'material.mp4',{type:'video/mp4'}),{silent:true});
    const source=PM.mkLayer('video',{name:'Video source',from:0,dur:4,d:{asset:asset.id,w:1920,h:1080,fit:'stretch',embeddedAudio:false}});
    source.on=false;PM.proj.layers.push(source);
    const api=PM.Kernel.api('video-material-performance');api.scene3d.edit({operation:'add_camera',camera:{p:{x:0,y:0,z:3}}});
    const imported=await PM.cmd('3d.import-obj',new File([new Uint8Array(model)],'screen.glb',{type:'model/gltf-binary'}));if(!imported.ok)throw new Error(imported.message);
    const receiver=PM.proj.layers.find((l:any)=>l.d?.data?.object).id;
    for(const slot of ['color','emissive']){const r=api.scene3d.setMaterialMap(receiver,0,slot,{layerId:source.id});if(!r.ok)throw new Error(r.message);}
    await PM.prepareFrame(0);PM.GL.render(0,{mblur:false});
    const capture=PM.GL.renderMaterialComposition,video=PM.GL.renderMaterialVideo;
    let captures=0,surfaces=0,seeks=0,playing=0;const sizes=new Set<string>();
    const seeking=()=>seeks++,played=()=>playing++;
    asset.el.addEventListener('seeking',seeking);asset.el.addEventListener('playing',played);
    PM.GL.renderMaterialComposition=(...args:any[])=>{captures++;return capture(...args);};
    PM.GL.renderMaterialVideo=(...args:any[])=>{const frame=video(...args);if(frame?.surface){surfaces++;sizes.add(`${frame.width}x${frame.height}`);}return frame;};
    try{
      PM.play();await new Promise(r=>setTimeout(r,1500));
      const streamed={captures,surfaces,seeks,playing,paused:asset.el.paused,at:asset.el.currentTime,sizes:[...sizes]};PM.pause();
      await PM.prepareFrame(.25,{mblur:false});
      const preview=()=>{
        PM.GL.render(.25,{mblur:false});
        const cv=document.createElement('canvas');cv.width=PM.GL.canvas.width;cv.height=PM.GL.canvas.height;
        const ctx=cv.getContext('2d')!;ctx.drawImage(PM.GL.canvas,0,0);return ctx.getImageData(0,0,cv.width,cv.height).data;
      };
      PM.touch();const livePixels=preview();
      PM.GL.renderMaterialVideo=()=>undefined;PM.touch();const reference=preview();
      let difference=0;for(let i=0;i<livePixels.length;i++)difference+=Math.abs(livePixels[i]-reference[i]);
      const previewError=difference/livePixels.length;
      PM.GL.renderMaterialVideo=video;PM.touch();
      // Rendering an exact prepared frame must retain timing, UV orientation
      // and color rather than using the latest live decoder surface.
      const pictures=[];
      for(const time of [.25,1.25]){
        await PM.prepareFrame(time,{mblur:false});
        const exact=PM.GL.renderToPixels(time,320,180,{transparent:true,mblur:false});
        // Disable only the fast path to produce an independent reference.
        PM.GL.renderMaterialVideo=()=>undefined;PM.touch();
        const baseline=PM.GL.renderToPixels(time,320,180,{transparent:true,mblur:false});
        let error=0;for(let i=0;i<exact.length;i++)error=Math.max(error,Math.abs(exact[i]-baseline[i]));
        pictures.push({error,sum:exact.reduce((a:number,b:number)=>a+b,0)});
        PM.GL.renderMaterialVideo=(...args:any[])=>video(...args);PM.touch();
      }
      return{streamed,previewError,pictures,errors:[...PM.GL.errors.entries()]};
    }finally{PM.pause();PM.GL.renderMaterialComposition=capture;PM.GL.renderMaterialVideo=video;asset.el.removeEventListener('seeking',seeking);asset.el.removeEventListener('playing',played);}
  },{movie:[...await readFile(movie)],model:[...screenGlb()]});
  await info.attach('video-material-performance',{body:Buffer.from(JSON.stringify(result,null,2)),contentType:'application/json'});
  expect(result.streamed.paused).toBe(false);expect(result.streamed.playing).toBeGreaterThan(0);
  expect(result.streamed.seeks).toBeLessThan(5);expect(result.streamed.at).toBeGreaterThan(.5);
  expect(result.streamed.surfaces).toBeGreaterThan(5);expect(result.streamed.captures).toBe(0);
  expect(result.previewError).toBeLessThan(3);
  expect(result.errors).toEqual([]);for(const picture of result.pictures)expect(picture.error).toBe(0);
  expect(result.pictures[0].sum).not.toBe(result.pictures[1].sum);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
