import { mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { expect, test } from './helpers/app';

test('saved render quality integrates native shutter samples and filters once',async({session})=>{
  test.setTimeout(120_000);await session.openEditor();
  await session.app.evaluate(({ipcMain})=>{
    const original=(ipcMain as any)._invokeHandlers.get('studio-native:request');
    (globalThis as any).__qualityRequests=[];
    ipcMain.removeHandler('studio-native:request');
    ipcMain.handle('studio-native:request',async(event,key,command,binary)=>{
      const reply=await original(event,key,command,binary);
      if(command.op==='render'&&command.shutter)(globalThis as any).__qualityRequests.push({command,result:reply.result});
      return reply;
    });
  });
  const result=await session.page.evaluate(async()=>{
    const PM=(window as any).PM;PM.pause();PM.AutomaticPreview?.cancel();PM.replaceProject(PM.mkProject({w:320,h:180,fps:30,dur:1}));
    const api=PM.Kernel.api('quality-test'),edit=(args:any)=>{const r=api.scene3d.edit(args);if(!r.ok)throw new Error(r.message);};
    edit({operation:'add_camera',camera:{p:{x:0,y:0,z:5}}});
    for(const [id,x,color] of [['a',-.7,'#FF0000'],['b',.7,'#00FF00']] as const){
      edit({operation:'add_object',id,object:{source:{primitive:'sphere'},p:{x},material:{p:{color,roughness:.6}}}});
      PM.L(id).mblur=true;PM.animate(PM.L(id),'position.x',[{t:0,v:x-.4},{t:1,v:x+.4}]);
    }
    const before=api.scene3d.renderQuality.get();
    const changed=api.scene3d.renderQuality.set({preset:'production',maxSamples:16,minSamples:8,timeLimitSeconds:0});if(!changed.ok)throw new Error(changed.message);
    const saved=api.scene3d.renderQuality.get();PM.hist.undo();const undone=api.scene3d.renderQuality.get();PM.hist.redo();PM.touch();
    PM.agentFrameCapture=true;
    try{
      await PM.prepareFrame(.5,{width:320,height:180,mblur:true,mbSamples:2,exporting:true});
      const c=PM.renderFrameTo(.5,320,180,{mblur:true,mbSamples:2,exporting:true}),pixels=c.getContext('2d').getImageData(0,0,320,180).data;
      let coloured=0;for(let i=0;i<pixels.length;i+=4)if(Math.max(pixels[i],pixels[i+1])-pixels[i+2]>20)coloured++;
      return {before,saved,undone,coloured,status:PM.Studio.status(),capture:PM.Studio.captureDiagnostics()};
    }finally{PM.agentFrameCapture=false;}
  });
  expect(result.before.preset).toBe('production');expect(result.undone).toEqual(result.before);
  expect(result.saved).toMatchObject({preset:'custom',maxSamples:16,minSamples:8});expect(result.coloured).toBeGreaterThan(500);
  expect(result.status.backend).toBe('native-studio');
  const requests=await session.app.evaluate(()=>(globalThis as any).__qualityRequests);
  expect(requests).toHaveLength(2);expect(requests.map((r:any)=>r.command.shutter.index)).toEqual([0,1]);
  expect(requests[0].result.deferred).toBe(true);expect(requests[1].result.denoiser).toBe('oidn');expect(requests[1].result.samples).toBe(16);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('measures a short glass export with the selected quality through the actual compositor',async({session})=>{
  test.skip(!process.env.PM_QUALITY_PROJECT,'Requires an explicit local verification project');test.setTimeout(600_000);
  const preset=process.env.PM_QUALITY_PRESET || 'production';
  if(!['draft','preview','production','final'].includes(preset))throw new Error('Choose a render quality preset');
  const out=path.resolve(process.env.PM_QUALITY_OUT || '/tmp/pm-shots/studio3d/laneQ/export');await mkdir(out,{recursive:true});
  await session.openEditor();
  await session.app.evaluate(({dialog,ipcMain},{project,out,preset})=>{
    dialog.showOpenDialog=async()=>({canceled:false,filePaths:[project]});
    dialog.showMessageBox=async()=>({response:1,checkboxChecked:false});
    dialog.showSaveDialog=async()=>({canceled:false,filePath:`${out}/glass-${preset}.mp4`});
    const original=(ipcMain as any)._invokeHandlers.get('studio-native:request');(globalThis as any).__qualityExport=[];
    ipcMain.removeHandler('studio-native:request');ipcMain.handle('studio-native:request',async(event,key,command,binary)=>{
      const reply=await original(event,key,command,binary);if(command.op==='render'&&command.tier==='final')(globalThis as any).__qualityExport.push({command,result:reply.result});return reply;
    });
  },{project:path.resolve(process.env.PM_QUALITY_PROJECT!),out,preset});
  const start=Date.now();
  const result=await session.page.evaluate(async(preset)=>{
    const PM=(window as any).PM;await PM.openProject();PM.pause();PM.AutomaticPreview?.cancel();PM.Studio?.setPreview('fast');
    PM.proj.work=[17.5,17.5+8/30];
    return PM.Export.run({format:'mp4',studioQuality:preset,fps:30,scale:1,range:'work',audio:false,mblur:true,quality:'max',reportPath:true});
  },preset);
  const requests=await session.app.evaluate(()=>(globalThis as any).__qualityExport);
  await writeFile(path.join(out,'export.json'),JSON.stringify({preset,seconds:(Date.now()-start)/1000,frames:8,result,requests},null,2));
  expect(result.error).toBeUndefined();expect(result.cancelled).toBe(false);expect(requests.some((r:any)=>r.command.shutter)).toBe(preset!=='draft');
});
