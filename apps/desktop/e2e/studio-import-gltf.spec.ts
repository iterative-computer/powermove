import { mkdir,readFile,writeFile } from 'node:fs/promises';
import path from 'node:path';
import { expect,test,repoRoot } from './helpers/app';
import { materialGltf } from './helpers/material-gltf';
const shots='/tmp/pm-shots/studio3d/laneK';

for(const transform of [false,true])test(`KTX2 Basis glTF renders ${transform?'transformed secondary UVs':'its four image quadrants'}`,async({session})=>{
  test.setTimeout(120_000);await mkdir(shots,{recursive:true});await session.openEditor();
  const image=await readFile(path.join(repoRoot,'e2e/fixtures/tiny-basis.ktx2'));
  const bytes=[...materialGltf(image,{basis:true,secondary:transform,transform})];
  const result=await session.page.evaluate(async bytes=>{
    const PM=(window as any).PM;PM.pause();PM.AutomaticPreview?.cancel();PM.replaceProject(PM.mkProject({name:'glTF material proof',w:640,h:360,fps:30,dur:1}));
    const api=PM.Kernel.api('gltf-e2e').scene3d;
    api.edit({operation:'add_camera',camera:{p:{x:0,y:0,z:3}}});
    const imported=await PM.cmd('3d.import-obj',new File([new Uint8Array(bytes)],'Material.gltf',{type:'model/gltf+json'}));
    if(!imported.ok)throw new Error(imported.message);
    await PM.prepareFrame(0,{mblur:false});const canvas=PM.renderFrameTo(0,640,360,{mblur:false}),ctx=canvas.getContext('2d');
    const pixels=[[290,150],[350,150],[290,210],[350,210]].map(([x,y])=>[...ctx.getImageData(x,y,1,1).data]);
    return {pixels,png:canvas.toDataURL('image/png'),status:PM.Studio.status()};
  },bytes);
  await writeFile(`${shots}/gltf-basis-${transform?'uv1-transform':'quadrants'}.png`,Buffer.from(result.png.split(',')[1],'base64'));
  const red=(p:number[])=>p[0]!>p[1]!+100 && p[0]!>p[2]!+100;
  const green=(p:number[])=>p[1]!>p[0]!+100 && p[1]!>p[2]!+100;
  if(transform)for(const pixel of result.pixels)expect(green(pixel)).toBe(true);
  else {expect(red(result.pixels[0]!)).toBe(true);expect(green(result.pixels[1]!)).toBe(true);expect(result.pixels[2]![2]).toBeGreaterThan(180);expect(Math.min(result.pixels[3]![0]!,result.pixels[3]![1]!)).toBeGreaterThan(180);}
  expect(session.diagnostics.pageErrors).toEqual([]);
  if(process.platform==='darwin')expect(result.status.backend).toBe('native-studio');
});
