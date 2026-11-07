import {mkdir,readFile,writeFile} from 'node:fs/promises';
import {execFileSync} from 'node:child_process';
import path from 'node:path';
import {test,expect,repoRoot} from './helpers/app';
import {screenGlb} from '../src/renderer/src/core/scene3d/__fixtures__/screen-glb';
const shots='/tmp/pm-figuate-feedback/shots/layer-materials-engine';
test.beforeAll(async()=>{await mkdir(shots,{recursive:true});});
async function setup(session:any){
  await session.openEditor();
  return session.page.evaluate(async(bytes:number[])=>{
    const PM=(window as any).PM;PM.pause();PM.AutomaticPreview?.cancel();
    PM.replaceProject(PM.mkProject({name:'Layer material proof',w:640,h:360,fps:30,dur:3}));
    const api=PM.Kernel.api('layer-material-proof');api.scene3d.edit({operation:'add_camera',camera:{p:{x:0,y:0,z:3}}});
    const imported=await PM.cmd('3d.import-obj',new File([new Uint8Array(bytes)],'screen.glb',{type:'model/gltf-binary'}));
    if(!imported.ok)throw new Error(imported.message);
    const id=PM.proj.layers.find((l:any)=>l.d?.data?.object).id;
    (window as any).materialSource=(source:string)=>{for(const slot of ['color','emissive']){const r=api.scene3d.setMaterialMap(id,0,slot,{layerId:source});if(!r.ok)throw new Error(r.message);}};
    return id;
  },[...screenGlb()]);
}
async function capture(page:any,time:number,name:string){
  const result=await page.evaluate(async(time:number)=>{
    const PM=(window as any).PM;await PM.prepareFrame(time,{mblur:false});
    const canvas=PM.renderFrameTo(time,640,360,{mblur:false}),ctx=canvas.getContext('2d'),data=ctx.getImageData(200,80,240,200).data;
    let green=0;for(let i=0;i<data.length;i+=4)if(data[i+1]>data[i]+30 && data[i+1]>data[i+2]+30)green++;
    return {png:canvas.toDataURL('image/png'),green,left:[...ctx.getImageData(270,180,1,1).data],right:[...ctx.getImageData(370,180,1,1).data],center:[...ctx.getImageData(320,180,1,1).data]};
  },time);
  await writeFile(`${shots}/${name}.png`,Buffer.from(result.png.split(',')[1],'base64'));return result;
}

test('hidden solid animates, includes masks/effects and ignores source transform and opacity',async({session})=>{
  const id=await setup(session),page=session.page;await capture(page,0,'01-before-original-screen');
  await page.evaluate(()=>{
    const PM=(window as any).PM;
    const result=PM.Edit.apply({type:'add_layer',id:'paint',layerType:'solid',name:'Animated hidden paint',visible:false,content:{w:128,h:128,color:'#FF0000'},properties:{opacity:0,'position.x':-1000,'scale.x':5,rotation:75}});if(!result.ok)throw new Error(result.message);
    const animated=PM.Edit.apply({type:'replace_keyframes',target:'paint',path:'c.color',keyframes:[{time:0,value:'#FF0000',hold:true},{time:1,value:'#00FF00',hold:true}]});if(!animated.ok)throw new Error(animated.message);
    PM.Edit.apply({type:'set_layer',target:'paint',patch:{threeD:true}});
    PM.Edit.apply({type:'set_property',target:'paint',path:'rotation.y',value:75});
    (window as any).materialSource('paint');
  });
  const red=await capture(page,0,'02-after-hidden-solid-red'),green=await capture(page,1,'03-after-hidden-solid-animated-green');
  expect(red.center[0]).toBeGreaterThan(red.center[1]+30);expect(green.center[1]).toBeGreaterThan(green.center[0]+30);
  await page.evaluate(()=>{
    const PM=(window as any).PM,source=PM.L('paint'),mask=PM.mkMask('rect',PM.proj);
    mask.p.x.v=32;mask.p.y.v=64;mask.p.w.v=64;mask.p.h.v=128;mask.p.feather.v=0;
    source.masks=[mask];source.fx=[PM.mkEffect('invert',undefined,source)];PM.touch();
  });
  const masked=await capture(page,0,'04-after-source-mask-and-invert');
  expect(masked.left[1]).toBeGreaterThan(100);expect(masked.left[2]).toBeGreaterThan(100);expect(masked.left[1]).toBeGreaterThan(masked.left[0]+30);
  expect(Math.max(...masked.right.slice(0,3))).toBeLessThan(10);
  const lifecycle=await page.evaluate((id:string)=>{
    const PM=(window as any).PM,api=PM.Kernel.api('layer-material-proof');PM.replaceProject(PM.deserialize(PM.serialize()));
    const stored=api.scene3d.materialLinks(id);PM.Edit.apply({type:'delete_layers',targets:['paint']});
    const missing=api.scene3d.materialLinks(id);PM.hist.undo();return {stored,missing,undo:api.scene3d.materialLinks(id)};
  },id);
  expect(lifecycle.stored[0].status).toBe('ready');expect(lifecycle.missing[0].status).toBe('missing');expect(lifecycle.undo[0].status).toBe('ready');
  const filePath=path.join(session.userData,'layer-material.pmv');
  await session.app.evaluate(({dialog},filePath)=>{dialog.showSaveDialog=async()=>({canceled:false,filePath});dialog.showOpenDialog=async()=>({canceled:false,filePaths:[filePath]});},filePath);
  await page.evaluate(async()=>{const PM=(window as any).PM;await PM.saveProject();await PM.openProject();});
  await expect.poll(()=>page.evaluate((id:string)=>{const PM=(window as any).PM;return !!PM.assets.get(PM.L(id)?.d?.data?.object?.source.assetId)?.object3d;},id)).toBe(true);
  const reopened=await capture(page,0,'05-after-reopen-and-undo');expect(reopened.left[1]).toBeGreaterThan(reopened.left[0]+30);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('animated precomp, text, shape and image sources render on the imported surface',async({session})=>{
  await setup(session);const page=session.page;
  await page.evaluate(()=>{
    const PM=(window as any).PM,main=PM.proj.compId,comp=PM.Comps.create({name:'Animation',w:128,h:128,fps:24,dur:3},{open:true});
    const r=PM.Edit.apply({type:'add_layer',id:'inside',layerType:'solid',content:{w:128,h:128,color:'#FF0000'},properties:{'position.x':64,'position.y':64,'anchor.x':64,'anchor.y':64}});if(!r.ok)throw new Error(r.message);
    PM.Edit.apply({type:'replace_keyframes',target:'inside',path:'c.color',keyframes:[{time:0,value:'#FF0000',hold:true},{time:1,value:'#0000FF',hold:true}]});
    PM.Edit.apply(['position.x','position.y'].map(path=>({type:'set_property',target:'inside',path,value:64})));
    PM.Comps.open(main);const add=PM.Edit.apply({type:'add_layer',id:'nested-source',layerType:'precomp',visible:false,content:{comp,w:128,h:128}});if(!add.ok)throw new Error(add.message);
    (window as any).materialSource('nested-source');
  });
  const red=await capture(page,0,'06-after-precomp-red'),blue=await capture(page,1,'07-after-precomp-animated-blue');
  expect(red.center[0]).toBeGreaterThan(red.center[2]+30);expect(blue.center[2]).toBeGreaterThan(blue.center[0]+30);
  await page.evaluate(()=>{
    const PM=(window as any).PM,result=PM.Edit.apply({type:'add_layer',id:'text-source',layerType:'text',visible:false,content:{text:'LIVE',size:72,color:'#00FF00'}});if(!result.ok)throw new Error(result.message);
    (window as any).materialSource('text-source');
  });
  const text=await capture(page,0,'08-after-text-layer');expect(text.green).toBeGreaterThan(100);
  await page.evaluate(()=>{const PM=(window as any).PM;PM.Edit.apply({type:'add_layer',id:'shape-source',layerType:'shape',visible:false,content:{w:128,h:128,shape:'ellipse',color:'#FF00FF'}});(window as any).materialSource('shape-source');});
  const shape=await capture(page,0,'09-after-shape-layer');expect(shape.center[0]).toBeGreaterThan(100);expect(shape.center[2]).toBeGreaterThan(100);
  await page.evaluate(async()=>{
    const PM=(window as any).PM,canvas=document.createElement('canvas');canvas.width=canvas.height=8;const ctx=canvas.getContext('2d')!;ctx.fillStyle='#FFFF00';ctx.fillRect(0,0,8,8);
    const blob=await new Promise<Blob>(resolve=>canvas.toBlob(b=>resolve(b!))),asset=await PM.assets.add(new File([blob],'yellow.png',{type:'image/png'}),{silent:true});
    PM.Edit.apply({type:'add_layer',id:'image-source',layerType:'image',visible:false,content:{asset:asset.id,w:128,h:128,fit:'stretch'}});(window as any).materialSource('image-source');
  });
  const image=await capture(page,0,'10-after-image-layer');expect(image.center[0]).toBeGreaterThan(100);expect(image.center[1]).toBeGreaterThan(100);expect(image.center[0]).toBeGreaterThan(image.center[2]+30);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('hidden video uses source in point, speed and time remap at receiver comp time',async({session})=>{
  await setup(session);const file=path.join(session.userData,'material.webm');
  const frames=Buffer.concat(Array.from({length:20},(_,i)=>{const frame=Buffer.alloc(64*64*3);for(let p=0;p<frame.length;p+=3)frame[p+(i<10?0:1)]=255;return frame;}));
  execFileSync(path.join(repoRoot,'node_modules/ffmpeg-static/ffmpeg'),['-v','error','-f','rawvideo','-pix_fmt','rgb24','-s','64x64','-r','10','-i','pipe:0','-c:v','libvpx-vp9','-lossless','1','-g','1',file],{input:frames});
  await session.page.evaluate(async(bytes:number[])=>{
    const PM=(window as any).PM,asset=await PM.assets.add(new File([new Uint8Array(bytes)],'material.webm',{type:'video/webm'}),{silent:true});
    const r=PM.Edit.apply({type:'add_layer',id:'video-source',layerType:'video',visible:false,from:.5,duration:2,content:{asset:asset.id,w:64,h:64,fit:'stretch',speed:2}});if(!r.ok)throw new Error(r.message);(window as any).materialSource('video-source');
  },[...await readFile(file)]);
  const red=await capture(session.page,.5,'11-after-hidden-video-red'),green=await capture(session.page,1.1,'12-after-hidden-video-green');
  expect(red.center[0]).toBeGreaterThan(red.center[1]+30);expect(green.center[1]).toBeGreaterThan(green.center[0]+30);
  await session.page.evaluate(()=>{const PM=(window as any).PM,r=PM.Edit.apply({type:'set_content',target:'video-source',patch:{timeRemap:true,sourceTime:0}});if(!r.ok)throw new Error(r.message);});
  const remapped=await capture(session.page,1.1,'13-after-video-remapped-red');expect(remapped.center[0]).toBeGreaterThan(remapped.center[1]+30);
  const outside=await capture(session.page,.25,'18-after-source-before-in-point');expect(Math.max(...outside.center.slice(0,3))).toBeLessThan(10);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('intrinsic emission and keyframed opacity change rendered surfaces',async({session})=>{
  const id=await setup(session);
  await session.page.evaluate((id:string)=>{
    const PM=(window as any).PM,r=PM.Kernel.api('layer-material-proof').scene3d.setMaterialProperties(id,0,{emissionColor:'#FF0000',emissionIntensity:4,opacity:{v:100,kf:[{t:0,v:100},{t:1,v:0}],expr:null},roughness:.1,metallic:0,ambientResponse:100});if(!r.ok)throw new Error(r.message);
  },id);
  const visible=await capture(session.page,0,'14-after-intrinsic-emission'),invisible=await capture(session.page,1,'15-after-keyframed-opacity-zero');
  expect(visible.center[0]).toBeGreaterThan(visible.center[1]+30);expect(Math.max(...invisible.center.slice(0,3))).toBeLessThan(10);
  await session.page.evaluate((id:string)=>{const PM=(window as any).PM,api=PM.Kernel.api('layer-material-proof');
    api.scene3d.edit({operation:'add_light',light:{p:{intensity:0}}});
    api.scene3d.edit({operation:'set_environment',patch:{p:{ambient:1}}});
    const r=api.scene3d.setMaterialProperties(id,0,{opacity:100,emissionIntensity:0,ambientResponse:100});if(!r.ok)throw new Error(r.message);
  },id);
  const ambient=await capture(session.page,0,'16-after-ambient-response-100');
  await session.page.evaluate((id:string)=>(window as any).PM.Kernel.api('layer-material-proof').scene3d.setMaterialProperties(id,0,{ambientResponse:0}),id);
  const noAmbient=await capture(session.page,0,'17-after-ambient-response-zero');
  expect(ambient.center[0]).toBeGreaterThan(noAmbient.center[0]+50);expect(noAmbient.center[0]).toBeLessThan(10);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
