import {mkdir,writeFile} from 'node:fs/promises';
import {expect,test} from './helpers/app';
test('Studio preserves orthographic projection, zoom and camera clip planes',async({session})=>{
 await session.openEditor();const folder='/tmp/pm-shots/studio3d/laneA';await mkdir(folder,{recursive:true});
 await session.page.evaluate(()=>{
  const PM=(window as any).PM;PM.pause();PM.AutomaticPreview?.cancel();PM.replaceProject(PM.mkProject({w:640,h:360,fps:30,dur:3}));
  const api=PM.Kernel.api('studio-camera').scene3d;
  api.edit({operation:'add_camera',camera:{p:{x:0,y:0,z:6}}});
  api.edit({operation:'add_object',object:{id:'model',source:{primitive:'box'},material:{p:{color:'#000000',emissive:'#FF0000',emissiveIntensity:1}}}});
 });
 const capture=async(name:string,patch?:any)=>{
  const r=await session.page.evaluate(async(patch:any)=>{
   const PM=(window as any).PM,api=PM.Kernel.api('studio-camera').scene3d;if(patch){const r=api.edit({operation:'set_camera',patch});if(!r.ok)throw new Error(r.message);}
   await PM.prepareFrame(0,{mblur:false,width:640,height:360});const c=PM.renderFrameTo(0,640,360,{mblur:false}),data=c.getContext('2d').getImageData(0,0,640,360).data;let red=0;for(let i=0;i<data.length;i+=4)if(data[i]>data[i+1]+40)red++;
   return {red,png:c.toDataURL()};
  },patch);
  await writeFile(`${folder}/camera-${name}.png`,Buffer.from(r.png.split(',')[1],'base64'));return r.red;
 };
 expect(await capture('01-perspective')).toBeGreaterThan(1000);
 expect(await capture('02-far-clipped',{p:{far:4}})).toBe(0);
 expect(await capture('03-near-clipped',{p:{far:1000,near:7}})).toBe(0);
 const ortho=await capture('04-orthographic',{projection:'orthographic',p:{near:.01,far:1000,zoom:1}});expect(ortho).toBeGreaterThan(1000);
 await session.page.evaluate(()=>(window as any).PM.Kernel.api('studio-camera').scene3d.edit({operation:'update_object',target:'model',patch:{p:{z:-5}}}));
 expect(await capture('05-orthographic-distant')).toBeCloseTo(ortho,-2);
 expect(await capture('06-orthographic-zoom',{p:{zoom:2}})).toBeGreaterThan(ortho*3.5);
 expect(session.diagnostics.pageErrors).toEqual([]);
});
