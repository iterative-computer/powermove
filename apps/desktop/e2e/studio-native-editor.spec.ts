import {expect,test} from './helpers/app';

test('Final quality reaches a 3D shot nested in a 2D composition',async({session})=>{
 test.setTimeout(120_000);await session.openEditor();
 const result=await session.page.evaluate(async()=>{
  const PM=(window as any).PM;PM.pause();PM.AutomaticPreview?.cancel();PM.replaceProject(PM.mkProject({w:640,h:360,fps:30,dur:3}));
  const main=PM.Comps.active(),shot=PM.Comps.create({name:'3D shot',w:640,h:360,dur:3},{open:true}),api=PM.Kernel.api('nested-final');
  api.scene3d.edit({operation:'add_camera',camera:{p:{x:0,y:0,z:5}}});
  api.scene3d.edit({operation:'add_object',object:{id:'shot-model',source:{primitive:'sphere'}}});
  api.scene3d.edit({operation:'set_environment',patch:{samples:16}});
  PM.Comps.open(main);PM.addLayer(PM.mkLayer('precomp',{from:0,dur:3,d:{comp:shot}}),0);PM.touch();
  await PM.prepareFrame(0,{exporting:true,mblur:false,width:640,height:360});PM.renderFrameTo(0,640,360,{exporting:true,mblur:false});
  return PM.Studio.status();
 });
 expect(result.frameSamples).toBeGreaterThanOrEqual(16);expect(session.diagnostics.pageErrors).toEqual([]);
});

test('the editor renders native Studio frames and keeps new depth sets native',async({session})=>{
 test.skip(process.platform!=='darwin','The native host is currently available on macOS.');
 test.setTimeout(120_000);await session.openEditor();const {page}=session;
 await session.app.evaluate(({sharedTexture})=>{
  const send=sharedTexture.sendSharedTexture;(globalThis as any).__studioSharedTransfers=0;
  (sharedTexture as any).sendSharedTexture=(...args:any[])=>{(globalThis as any).__studioSharedTransfers++;return Reflect.apply(send,sharedTexture,args);};
 });
 await page.evaluate(()=>{
  const PM=(window as any).PM;PM.pause();PM.AutomaticPreview?.cancel();PM.replaceProject(PM.mkProject({w:640,h:360,fps:30,dur:3}));
  const api=PM.Kernel.api('native-editor');
  api.scene3d.edit({operation:'add_camera',camera:{p:{x:0,y:0,z:5}}});
  api.scene3d.edit({operation:'add_object',object:{id:'native-model',source:{primitive:'sphere'},material:{p:{color:'#ff0000',roughness:.5}}}});
 });
 await page.evaluate(()=>(window as any).PM.Studio.setPreview('fast'));
 const capture=()=>page.evaluate(async()=>{
  const PM=(window as any).PM;PM.agentFrameCapture=true;await PM.prepareFrame(0,{mblur:false,width:640,height:360});
  const c=PM.renderFrameTo(0,640,360,{mblur:false}),px=c.getContext('2d').getImageData(0,0,640,360).data;PM.agentFrameCapture=false;
  let red=0;for(let i=0;i<px.length;i+=4)if(px[i]>px[i+1]+30&&px[i]>px[i+2]+30)red++;
  return {red,backend:PM.Studio.status().backend};
 });
 await expect.poll(()=>page.evaluate(()=>(window as any).PM.Studio?.status().ready),{timeout:30_000}).toBe(true);
 await expect.poll(()=>session.app.evaluate(()=>(globalThis as any).__studioSharedTransfers),{timeout:30_000}).toBeGreaterThan(0);
 const first=await capture();expect(first.backend).toBe('native-studio');expect(first.red).toBeGreaterThan(200);
 for(let i=0;i<3;i++){
  await page.evaluate(i=>{const PM=(window as any).PM;PM.Kernel.api('native-editor').scene3d.edit({operation:'add_object',object:{id:`added-${i}`,source:{primitive:'box',parameters:{width:.3,height:.3,depth:.3}},p:{x:i*.4-.4,y:1}}});},i);
  const result=await capture();expect(result.backend).toBe('native-studio');expect(result.red).toBeGreaterThan(200);
 }
 expect(await session.app.evaluate(()=>(globalThis as any).__studioSharedTransfers)).toBeGreaterThan(0);
 expect(session.diagnostics.pageErrors).toEqual([]);
});
