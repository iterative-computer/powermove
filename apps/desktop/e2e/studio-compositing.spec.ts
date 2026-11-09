import {mkdir,writeFile} from 'node:fs/promises';
import {expect,test} from './helpers/app';
test('layer opacity and masks reveal the 3D layers behind them',async({session})=>{
 await session.openEditor();await session.page.evaluate(()=>{
  const PM=(window as any).PM;PM.pause();PM.AutomaticPreview?.cancel();PM.replaceProject(PM.mkProject({w:640,h:360,fps:30,dur:3}));const api=PM.Kernel.api('studio-compositing').scene3d;
  api.edit({operation:'add_camera',camera:{p:{x:0,y:0,z:5}}});
  for(const [id,z,color] of [['back',0,'#0000FF'],['front',1,'#FF0000']]){const r=api.edit({operation:'add_object',object:{id,source:{primitive:'plane',parameters:{width:2,height:2}},p:{z},material:{p:{color:'#000000',emissive:color,emissiveIntensity:1}}}});if(!r.ok)throw new Error(r.message);}
  PM.Edit.apply({type:'set_property',target:'front',path:'opacity',value:50});
 });
 const capture=async(name:string)=>{const r=await session.page.evaluate(async()=>{
  const PM=(window as any).PM;PM.agentFrameCapture=true;await PM.prepareFrame(0,{mblur:false,width:640,height:360});const c=PM.renderFrameTo(0,640,360,{mblur:false}),ctx=c.getContext('2d'),data=ctx.getImageData(0,0,640,360).data;PM.agentFrameCapture=false;let red=0,blue=0;for(let i=0;i<data.length;i+=4){if(data[i]>data[i+2]+40)red++;if(data[i+2]>data[i]+40)blue++;}return {center:[...ctx.getImageData(320,180,1,1).data],red,blue,png:c.toDataURL()};
 });await mkdir('/tmp/pm-shots/studio3d/laneA',{recursive:true});await writeFile(`/tmp/pm-shots/studio3d/laneA/compositing-${name}.png`,Buffer.from(r.png.split(',')[1],'base64'));return r;};
 const opacity=await capture('01-opacity');expect(opacity.center[0]).toBeGreaterThan(60);expect(opacity.center[2]).toBeGreaterThan(60);
 await session.page.evaluate(()=>{const PM=(window as any).PM;PM.Edit.apply({type:'set_property',target:'front',path:'opacity',value:100});const mask=PM.mkMask('rect',PM.proj);mask.p.x.v=320;mask.p.y.v=180;mask.p.w.v=100;mask.p.h.v=360;mask.p.feather.v=0;PM.L('front').masks=[mask];PM.touch();});
 const masked=await capture('02-mask');expect(masked.red).toBeGreaterThan(100);expect(masked.blue).toBeGreaterThan(100);
 expect(session.diagnostics.pageErrors).toEqual([]);
});
