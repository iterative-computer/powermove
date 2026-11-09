import {mkdir,writeFile} from 'node:fs/promises';
import {expect,test} from './helpers/app';
function instanceGlb(){
 const positions=new Float32Array([-1,-.7,0,1,-.7,0,1,.7,0,-1,-.7,0,1,.7,0,-1,.7,0]),translations=new Float32Array(27),colors=new Float32Array(27);
 for(let i=0;i<9;i++){translations.set([(i%3-1)*3,(Math.floor(i/3)-1)*2,0],i*3);colors.set([i%3===0?1:0,i%3===1?1:0,i%3===2?1:0],i*3);}
 const bin=Buffer.concat([Buffer.from(positions.buffer),Buffer.from(translations.buffer),Buffer.from(colors.buffer)]);
 const json={asset:{version:'2.0'},extensionsUsed:['EXT_mesh_gpu_instancing','KHR_materials_unlit'],extensionsRequired:['EXT_mesh_gpu_instancing'],scene:0,scenes:[{nodes:[0]}],nodes:[{name:'Repeated props',mesh:0,extensions:{EXT_mesh_gpu_instancing:{attributes:{TRANSLATION:1,_COLOR_0:2}}}}],meshes:[{primitives:[{attributes:{POSITION:0},material:0}]}],materials:[{name:'Instance colors',doubleSided:true,extensions:{KHR_materials_unlit:{}},pbrMetallicRoughness:{baseColorFactor:[1,1,1,1]}}],buffers:[{byteLength:bin.length}],bufferViews:[{buffer:0,byteOffset:0,byteLength:positions.byteLength},{buffer:0,byteOffset:positions.byteLength,byteLength:translations.byteLength},{buffer:0,byteOffset:positions.byteLength+translations.byteLength,byteLength:colors.byteLength}],accessors:[{bufferView:0,componentType:5126,count:6,type:'VEC3',min:[-1,-.7,0],max:[1,.7,0]},{bufferView:1,componentType:5126,count:9,type:'VEC3'},{bufferView:2,componentType:5126,count:9,type:'VEC3'}]};
 const raw=Buffer.from(JSON.stringify(json)),text=Buffer.concat([raw,Buffer.alloc((4-raw.length%4)%4,32)]),header=Buffer.alloc(20);header.writeUInt32LE(0x46546c67,0);header.writeUInt32LE(2,4);header.writeUInt32LE(28+text.length+bin.length,8);header.writeUInt32LE(text.length,12);header.writeUInt32LE(0x4e4f534a,16);const bh=Buffer.alloc(8);bh.writeUInt32LE(bin.length,0);bh.writeUInt32LE(0x004e4942,4);return Buffer.concat([header,text,bh,bin]);
}
test('imported glTF instances share geometry and keep their individual colors',async({session})=>{
 await session.openEditor();const result=await session.page.evaluate(async(bytes:number[])=>{
  const PM=(window as any).PM;PM.pause();PM.AutomaticPreview?.cancel();PM.replaceProject(PM.mkProject({w:640,h:360,fps:30,dur:3}));const api=PM.Kernel.api('studio-instances').scene3d;api.edit({operation:'add_camera',camera:{p:{x:0,y:0,z:3}}});
  const imported=await PM.cmd('3d.import-obj',new File([new Uint8Array(bytes)],'repeated-props.glb',{type:'model/gltf-binary'}));if(!imported.ok)throw new Error(imported.message);
  PM.agentFrameCapture=true;await PM.prepareFrame(0,{mblur:false,width:640,height:360});const c=PM.renderFrameTo(0,640,360,{mblur:false}),pixels=c.getContext('2d').getImageData(0,0,640,360).data;PM.agentFrameCapture=false;let red=0,green=0,blue=0;for(let i=0;i<pixels.length;i+=4){if(pixels[i]>pixels[i+1]+60&&pixels[i]>pixels[i+2]+60)red++;if(pixels[i+1]>pixels[i]+60&&pixels[i+1]>pixels[i+2]+60)green++;if(pixels[i+2]>pixels[i]+60&&pixels[i+2]>pixels[i+1]+60)blue++;}
  const layer=PM.L(imported.data.result.id),asset=PM.assets.get(layer.d.data.object.source.assetId);let instances=0;asset.object3d.traverse((o:any)=>{if(o.isInstancedMesh)instances+=o.count;});return {instances,red,green,blue,png:c.toDataURL()};
 },[...instanceGlb()]);
 expect(result.instances).toBe(9);expect(Math.min(result.red,result.green,result.blue)).toBeGreaterThan(500);
 await mkdir('/tmp/pm-shots/studio3d/laneA',{recursive:true});await writeFile('/tmp/pm-shots/studio3d/laneA/studio-instanced-import.png',Buffer.from(result.png.split(',')[1],'base64'));
 expect(session.diagnostics.pageErrors).toEqual([]);
});
