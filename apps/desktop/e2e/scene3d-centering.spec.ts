import { mkdir, writeFile } from 'node:fs/promises';
import { expect, test } from './helpers/app';

// A self-contained GLB with a deliberately off-origin pivot and a flat front.
function modelGlb(): number[] {
  const positions = new Float32Array([-2,-1,0, 2,-1,0, 2,1,0, -2,-1,0, 2,1,0, -2,1,0]);
  const json = JSON.stringify({asset:{version:'2.0'},scene:0,scenes:[{nodes:[0]}],nodes:[{mesh:0,translation:[8,4,-3]}],
    meshes:[{primitives:[{attributes:{POSITION:0}}]}],buffers:[{byteLength:positions.byteLength}],
    bufferViews:[{buffer:0,byteOffset:0,byteLength:positions.byteLength}],
    accessors:[{bufferView:0,componentType:5126,count:6,type:'VEC3',min:[-2,-1,0],max:[2,1,0]}]});
  const text = new TextEncoder().encode(json.padEnd(Math.ceil(json.length/4)*4,' '));
  const bytes = new Uint8Array(28+text.length+positions.byteLength), view=new DataView(bytes.buffer);
  view.setUint32(0,0x46546c67,true);view.setUint32(4,2,true);view.setUint32(8,bytes.length,true);
  view.setUint32(12,text.length,true);view.setUint32(16,0x4e4f534a,true);bytes.set(text,20);
  view.setUint32(20+text.length,positions.byteLength,true);view.setUint32(24+text.length,0x004e4942,true);
  bytes.set(new Uint8Array(positions.buffer),28+text.length);return [...bytes];
}

test('fresh GLB sets up Studio and implicit old/new camera defaults survive save/load', async ({session}) => {
  await session.openEditor();
  const result=await session.page.evaluate(async bytes=>{
    const PM=(window as any).PM,api=PM.Kernel.api('centering-proof');
    const imported=await PM.cmd('3d.import-obj',new File([new Uint8Array(bytes)],'off-origin.glb',{type:'model/gltf-binary'}));
    if(!imported.ok)throw new Error(imported.message);
    const id=imported.data.result.id;
    const studioLayers=PM.proj.layers.filter((l:any)=>l.d?.data?.camera || l.d?.data?.light);
    const autoStudio={cameras:studioLayers.filter((l:any)=>l.d?.data?.camera).length,lights:studioLayers.filter((l:any)=>l.d?.data?.light).length};
    // Removing the automatic rig retains main's implicit-camera capability.
    for(const layer of studioLayers)api.scene3d.edit({operation:'remove',target:layer.id});
    api.scene3d.edit({operation:'update_object',target:id,patch:{p:{x:0,y:0,z:0}}});
    const capture=()=>{
      const w=640,h=360,pixels=PM.GL.renderToPixels(0,w,h,{transparent:true,mblur:false});
      const canvas=document.createElement('canvas');canvas.width=w;canvas.height=h;
      const top=new Uint8ClampedArray(pixels.length);
      for(let y=0;y<h;y++)top.set(pixels.subarray(y*w*4,(y+1)*w*4),(h-1-y)*w*4);
      canvas.getContext('2d')!.putImageData(new ImageData(top,w,h),0,0);
      return {png:canvas.toDataURL(),pixels:[...pixels]};
    };
    const implicit=capture(),scene=api.scene3d.describe(),saved=PM.serialize();
    const cameraCount=PM.proj.layers.filter((l:any)=>l.d?.definition==='powermove.3d.camera').length;
    api.scene3d.edit({operation:'add_camera'});const explicit=capture();PM.hist.undo();
    PM.replaceProject(PM.hydrateProject(PM.deserialize(saved)));const restored=capture();
    api.scene3d.edit({operation:'update_object',target:id,patch:{p:{x:.5}}});const moved=capture();PM.hist.undo();
    delete PM.proj.scene3dDefaultsVersion;PM.touch();const legacy=capture(),legacySaved=PM.serialize();
    PM.replaceProject(PM.hydrateProject(PM.deserialize(legacySaved)));const legacyRestored=capture();
    const oldCamera=api.scene3d.describe().camera.p;
    api.scene3d.edit({operation:'add_camera',camera:{p:{x:3,y:2,z:6}}});const legacyExplicit=capture();
    const xs:number[]=[],ys:number[]=[];
    for(let y=0;y<360;y++)for(let x=0;x<640;x++)if(implicit.pixels[(y*640+x)*4+3]>127){xs.push(x);ys.push(y);}
    return {autoStudio,png:implicit.png,explicitPng:explicit.png,movedPng:moved.png,legacyPng:legacyRestored.png,
      identical:JSON.stringify(implicit.pixels)===JSON.stringify(explicit.pixels),
      roundtrip:JSON.stringify(implicit.pixels)===JSON.stringify(restored.pixels),
      legacyRoundtrip:JSON.stringify(legacy.pixels)===JSON.stringify(legacyRestored.pixels),
      legacyExplicit:JSON.stringify(legacy.pixels)===JSON.stringify(legacyExplicit.pixels),oldCamera:[oldCamera.x.v,oldCamera.y.v,oldCamera.z.v],
      bounds:[Math.min(...xs),Math.min(...ys),Math.max(...xs),Math.max(...ys)],
      camera:[scene.camera.p.x.v,scene.camera.p.y.v,scene.camera.p.z.v],cameraCount,saved,position:['x','y','z'].map(axis=>PM.L(id).p[`position.${axis}`].v)};
  },modelGlb());
  const dir='/tmp/pm-figuate-feedback/shots/3d-engine';await mkdir(dir,{recursive:true});
  await writeFile(`${dir}/02-after-fresh-glb.png`,Buffer.from(result.png.split(',')[1],'base64'));
  await writeFile(`${dir}/03-after-explicit-camera.png`,Buffer.from(result.explicitPng.split(',')[1],'base64'));
  await writeFile(`${dir}/04-after-position-x.png`,Buffer.from(result.movedPng.split(',')[1],'base64'));
  await writeFile(`${dir}/05-after-legacy-preserved.png`,Buffer.from(result.legacyPng.split(',')[1],'base64'));
  expect(result.autoStudio).toEqual({cameras:1,lights:3});
  expect(result.position).toEqual([0,0,0]);expect(result.cameraCount).toBe(0);
  expect(result.camera).toEqual([0,0,6]);expect(result.identical).toBe(true);
  expect(result.roundtrip).toBe(true);expect(result.legacyRoundtrip).toBe(true);expect(result.legacyExplicit).toBe(true);
  expect(result.oldCamera).toEqual([3,2,6]);
  expect(result.bounds[0]+result.bounds[2]).toBeCloseTo(639,0);
  expect(result.bounds[1]+result.bounds[3]).toBeCloseTo(359,0);
  expect(JSON.parse(result.saved).proj.scene3dDefaultsVersion).toBe(2);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
