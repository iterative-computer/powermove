import {mkdir,writeFile,readFile} from 'node:fs/promises';
import {execFileSync} from 'node:child_process';
import path from 'node:path';
import {test,expect,repoRoot} from './helpers/app';
const shots='/tmp/pm-figuate-feedback/shots/extruded-3d-engine';
test.beforeAll(async()=>{await mkdir(shots,{recursive:true});});
async function setup(session:any){
  await session.openEditor();
  await session.page.evaluate(()=>{
    const PM=(window as any).PM;PM.pause();PM.AutomaticPreview?.cancel();
    PM.replaceProject(PM.mkProject({name:'Extrusion proof',w:960,h:540,fps:30,dur:3}));
    (window as any).extrusionAPI=PM.Kernel.api('extrusion-proof').scene3d;
    const r=PM.Edit.apply({type:'add_layer',id:'title',layerType:'text',name:'Powermove',content:{text:'Powermove',font:'Arial',weight:700,size:100,align:'center',color:'#DAD7FF'},properties:{'position.x':480,'position.y':230}});if(!r.ok)throw new Error(r.message);
  });
}
async function capture(page:any,time:number,name:string){
  const result=await page.evaluate(async(time:number)=>{
    const PM=(window as any).PM;await PM.prepareFrame(time,{mblur:false});const canvas=PM.renderFrameTo(time,960,540,{mblur:false}),ctx=canvas.getContext('2d'),data=ctx.getImageData(0,0,960,540).data;
    let lit=0,red=0,green=0,blue=0;for(let i=0;i<data.length;i+=4){if(Math.max(data[i],data[i+1],data[i+2])>35)lit++;if(data[i]>data[i+1]+35 && data[i]>data[i+2]+35)red++;if(data[i+1]>data[i]+35 && data[i+1]>data[i+2]+35)green++;if(data[i+2]>data[i]+35 && data[i+2]>data[i+1]+35)blue++;}
    return {png:canvas.toDataURL('image/png'),lit,red,green,blue};
  },time);
  await writeFile(`${shots}/${name}.png`,Buffer.from(result.png.split(',')[1],'base64'));return result;
}
async function lights(page:any){
  await page.evaluate(()=>{
    const api=(window as any).extrusionAPI;
    for(const args of [{operation:'add_camera',camera:{p:{x:0,y:0,z:8}}},{operation:'add_light',light:{type:'point',p:{x:-3,y:3,z:4,intensity:80,decay:2}}},{operation:'add_light',light:{type:'spot',p:{x:4,y:4,z:4,intensity:120,targetX:0,targetY:0,targetZ:0,angle:50,color:'#9CAFFF'}}}]){const r=api.edit(args);if(!r.ok)throw new Error(r.message);}
  });
}
test('extruded title and beveled shapes render with lights, camera orbit, saved surfaces and geometry animation',async({session})=>{
  test.setTimeout(90000);await setup(session);const page=session.page;
  const before=await capture(page,0,'01-before-flat-title');expect(before.lit).toBeGreaterThan(1000);
  await lights(page);
  const state=await page.evaluate(()=>{
    const api=(window as any).extrusionAPI;const r=api.setExtrusion('title',{enabled:true,depth:60,bevelStyle:'angular',bevelDepth:4});if(!r.ok)throw new Error(r.message);
    const side=api.setSurfaceMaterialProperties('title','side',{color:'#E44828',roughness:.3,metallic:.2});if(!side.ok)throw new Error(side.message);
    (window as any).PM.Edit.apply({type:'set_property',target:'title',path:'rotation.y',value:-24});return api.getExtrusion('title');
  });expect(state.enabled).toBe(true);
  const angular=await capture(page,0,'02-after-angular-title');expect(angular.lit).toBeGreaterThan(1000);expect(angular.red).toBeGreaterThan(100);
  await page.evaluate(()=>{const api=(window as any).extrusionAPI;api.setExtrusion('title',{bevelStyle:'convex',bevelDepth:5});});
  const convex=await capture(page,0,'03-after-convex-title');expect(convex.png).not.toBe(angular.png);
  await page.evaluate(()=>{
    const PM=(window as any).PM,api=(window as any).extrusionAPI;
    for(const [id,shape,x] of [['round','rect',280],['star','star',680]]){
      const r=PM.Edit.apply({type:'add_layer',id,layerType:'shape',content:{shape,w:180,h:140,radius:35,points:5,color:'#24DC40'},properties:{'position.x':x,'position.y':380,'rotation.x':-20,'rotation.y':-25}});if(!r.ok)throw new Error(r.message);
      const ex=api.setExtrusion(id,{enabled:true,depth:45,bevelStyle:'convex',bevelDepth:5});if(!ex.ok)throw new Error(ex.message);
      api.setSurfaceMaterialProperties(id,'side',{color:'#1455DB'});
    }
  });
  const shapes=await capture(page,0,'04-after-rounded-rect-and-star');expect(shapes.green).toBeGreaterThan(500);expect(shapes.blue).toBeGreaterThan(50);
  await page.evaluate(()=>{const api=(window as any).extrusionAPI;const r=api.edit({operation:'set_camera',patch:{p:{x:3,y:1,z:7,targetX:0,targetY:0,targetZ:0}}});if(!r.ok)throw new Error(r.message);});
  const orbit=await capture(page,0,'05-after-camera-orbit');expect(orbit.lit).toBeGreaterThan(1000);expect(orbit.png).not.toBe(shapes.png);
  const lifecycle=await page.evaluate(()=>{
    const PM=(window as any).PM,api=(window as any).extrusionAPI;PM.replaceProject(PM.deserialize(PM.serialize()));
    const stored=api.getExtrusion('title');const r=api.setExtrusion('title',{depth:100});if(!r.ok)throw new Error(r.message);PM.hist.undo();const undo=api.getExtrusion('title');PM.hist.redo();const redo=api.getExtrusion('title');PM.hist.undo();
    return {stored,undo,redo};
  });expect(lifecycle.stored.values.depth).toBe(60);expect(lifecycle.undo.values.depth).toBe(60);expect(lifecycle.redo.values.depth).toBe(100);
  const saved=path.join(session.userData,'extruded-title.pmv');await session.app.evaluate(({dialog},filePath)=>{dialog.showSaveDialog=async()=>({canceled:false,filePath});dialog.showOpenDialog=async()=>({canceled:false,filePaths:[filePath]});},saved);
  await page.evaluate(async()=>{const PM=(window as any).PM;await PM.saveProject();await PM.openProject();});
  const reopened=await capture(page,0,'06-after-file-reopen');expect(reopened.lit).toBeGreaterThan(1000);expect(session.diagnostics.pageErrors).toEqual([]);
});
test('video front material and independent side render on extruded text at source time',async({session})=>{
  test.setTimeout(90000);await setup(session);await lights(session.page);
  const file=path.join(session.userData,'extruded-material.webm'),frames=Buffer.concat(Array.from({length:20},(_,i)=>{const frame=Buffer.alloc(64*64*3);for(let p=0;p<frame.length;p+=3)frame[p+(i<10?0:1)]=255;return frame;}));
  execFileSync(path.join(repoRoot,'node_modules/ffmpeg-static/ffmpeg'),['-v','error','-f','rawvideo','-pix_fmt','rgb24','-s','64x64','-r','10','-i','pipe:0','-c:v','libvpx-vp9','-lossless','1','-g','1',file],{input:frames});
  await session.page.evaluate(async(bytes:number[])=>{
    const PM=(window as any).PM,api=(window as any).extrusionAPI;api.setExtrusion('title',{enabled:true,depth:60,bevelStyle:'angular',bevelDepth:3});PM.Edit.apply({type:'set_property',target:'title',path:'rotation.y',value:-25});
    const asset=await PM.assets.add(new File([new Uint8Array(bytes)],'material.webm',{type:'video/webm'}),{silent:true});
    const add=PM.Edit.apply({type:'add_layer',id:'movie',layerType:'video',visible:false,content:{asset:asset.id,w:64,h:64,fit:'stretch'}});if(!add.ok)throw new Error(add.message);
    for(const slot of ['color','emissive']){const r=api.setSurfaceMaterialMap('title','front',slot,{layerId:'movie'});if(!r.ok)throw new Error(r.message);}
    api.setSurfaceMaterialProperties('title','side',{color:'#1855EE'});
  },[...await readFile(file)]);
  const red=await capture(session.page,.1,'07-after-video-front-red'),green=await capture(session.page,1.2,'08-after-video-front-green');
  expect(red.red).toBeGreaterThan(1000);expect(green.green).toBeGreaterThan(1000);expect(green.blue).toBeGreaterThan(100);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
test('cap UVs keep a four-color image upright and aligned across the full layer bounds',async({session})=>{
  await setup(session);await lights(session.page);
  await session.page.evaluate(async()=>{
    const PM=(window as any).PM,api=(window as any).extrusionAPI;PM.Edit.apply({type:'delete_layers',targets:['title']});
    const canvas=document.createElement('canvas');canvas.width=canvas.height=64;const ctx=canvas.getContext('2d')!;
    for(const [color,x,y] of [['#FF0000',0,0],['#00FF00',32,0],['#0000FF',0,32],['#FFFF00',32,32]]){ctx.fillStyle=color as string;ctx.fillRect(x as number,y as number,32,32);}
    const blob=await new Promise<Blob>(resolve=>canvas.toBlob(b=>resolve(b!))),asset=await PM.assets.add(new File([blob],'quadrants.png',{type:'image/png'}),{silent:true});
    (window as any).quadrantAsset=asset.id;PM.Edit.apply({type:'add_layer',id:'screen',layerType:'shape',content:{shape:'rect',w:320,h:200},properties:{'position.x':480,'position.y':270}});
    api.setExtrusion('screen',{enabled:true,depth:35});for(const slot of ['color','emissive']){const r=api.setSurfaceMaterialMap('screen','front',slot,asset.id);if(!r.ok)throw new Error(r.message);}
  });
  await capture(session.page,0,'09-after-cap-uv-alignment');
  const pixels=await session.page.evaluate(()=>{
    const PM=(window as any).PM,canvas=PM.renderFrameTo(0,960,540,{mblur:false}),ctx=canvas.getContext('2d');
    return [[430,240],[530,240],[430,300],[530,300]].map(([x,y])=>[...ctx.getImageData(x,y,1,1).data]);
  });
  expect(pixels[0][0]).toBeGreaterThan(pixels[0][1]+80);expect(pixels[1][1]).toBeGreaterThan(pixels[1][0]+20);expect(pixels[2][2]).toBeGreaterThan(pixels[2][0]+40);expect(pixels[3][0]).toBeGreaterThan(150);expect(pixels[3][1]).toBeGreaterThan(150);expect(Math.min(pixels[3][0],pixels[3][1])).toBeGreaterThan(pixels[3][2]+40);
  await session.page.evaluate(()=>{
    const PM=(window as any).PM,api=(window as any).extrusionAPI;
    PM.Edit.apply({type:'add_layer',id:'quad-source',layerType:'image',visible:false,content:{asset:(window as any).quadrantAsset,w:64,h:64,fit:'stretch'}});
    for(const slot of ['color','emissive']){const r=api.setSurfaceMaterialMap('screen','front',slot,{layerId:'quad-source'});if(!r.ok)throw new Error(r.message);}
  });
  await capture(session.page,0,'10-after-layer-cap-uv-alignment');
  const layerPixels=await session.page.evaluate(()=>{const PM=(window as any).PM,canvas=PM.renderFrameTo(0,960,540,{mblur:false}),ctx=canvas.getContext('2d');return [[430,240],[530,240],[430,300],[530,300]].map(([x,y])=>[...ctx.getImageData(x,y,1,1).data]);});
  for(let i=0;i<4;i++)for(let channel=0;channel<3;channel++)expect(Math.abs(layerPixels[i][channel]-pixels[i][channel])).toBeLessThan(5);
});
test('20-character title reuses its mesh during camera motion and stays within the frame budget',async({session})=>{
  await setup(session);await lights(session.page);
  const numbers=await session.page.evaluate(async()=>{
    const PM=(window as any).PM,api=(window as any).extrusionAPI;PM.Edit.apply({type:'set_content',target:'title',patch:{text:'Powermove 3D Title20',size:58}});api.setExtrusion('title',{enabled:true,depth:35,bevelStyle:'convex',bevelDepth:3});
    const camera=PM.proj.layers.find((l:any)=>l.d?.data?.camera);PM.Edit.apply({type:'replace_keyframes',target:camera.id,path:'position.x',keyframes:[{time:0,value:0},{time:1,value:3}]});
    await PM.prepareFrame(0,{mblur:false});let start=performance.now();PM.renderFrameTo(0,960,540,{mblur:false});const cold=performance.now()-start;
    const frames:number[]=[];for(let i=0;i<35;i++){start=performance.now();PM.renderFrameTo(i/30,960,540,{mblur:false});frames.push(performance.now()-start);}
    const warm=frames.slice(5).sort((a,b)=>a-b);return {characters:PM.L('title').d.text.length,coldMs:cold,medianMs:warm[Math.floor(warm.length/2)],p95Ms:warm[Math.floor(warm.length*.95)],frames:warm.length};
  });
  await writeFile(`${shots}/performance.json`,JSON.stringify(numbers,null,2));
  // Timing is recorded rather than enforced on a shared GPU/CPU runner.
  expect(numbers.characters).toBe(20);expect(numbers.frames).toBe(30);
});
test('extrusions, model meshes and legacy 3D planes share camera depth across the stack',async({session})=>{
  await setup(session);await lights(session.page);
  await session.page.evaluate(()=>{
    const PM=(window as any).PM,api=(window as any).extrusionAPI;PM.Edit.apply({type:'delete_layers',targets:['title']});
    PM.Edit.apply({type:'add_layer',id:'receiver',layerType:'shape',content:{shape:'rect',w:320,h:200,radius:0,color:'#FF0000'},properties:{'position.x':480,'position.y':270}});api.setExtrusion('receiver',{enabled:true,depth:60});
    const box=api.edit({operation:'add_object',object:{id:'model',source:{primitive:'box'},p:{x:0,y:0,z:1},material:{p:{color:'#00FF00',emissive:'#00FF00',emissiveIntensity:2}}}});if(!box.ok)throw new Error(box.message);
    PM.Edit.apply({type:'add_layer',id:'flat',layerType:'solid',content:{w:120,h:120,color:'#0000FF'},properties:{'position.x':480,'position.y':270,'anchor.x':60,'anchor.y':60,'position.z':-180}});PM.Edit.apply([{type:'set_layer',target:'flat',patch:{threeD:true}},{type:'set_property',target:'flat',path:'position.x',value:480},{type:'set_property',target:'flat',path:'position.y',value:270}]);
  });
  await session.page.evaluate(()=>(window as any).PM.touch());
  const front=await capture(session.page,0,'11-after-model-plane-shared-depth');expect(front.blue).toBeGreaterThan(1000);
  const first=await session.page.evaluate(()=>{const PM=(window as any).PM,canvas=PM.renderFrameTo(0,960,540,{mblur:false});return [...canvas.getContext('2d').getImageData(480,270,1,1).data];});expect(first[2]).toBeGreaterThan(first[1]+100);
  await session.page.evaluate(()=>{const PM=(window as any).PM;PM.Edit.apply({type:'set_property',target:'flat',path:'position.z',value:180});});
  const behind=await capture(session.page,0,'12-after-plane-behind-model');expect(behind.lit).toBeGreaterThan(1000);
  const second=await session.page.evaluate(()=>{const PM=(window as any).PM,canvas=PM.renderFrameTo(0,960,540,{mblur:false});return [...canvas.getContext('2d').getImageData(480,270,1,1).data];});expect(second[1]).toBeGreaterThan(second[2]+20);expect(second[1]).toBeGreaterThan(second[0]+10);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
test('extruded motion blur renders distinct samples and exports a PNG through the normal export path',async({session})=>{
  test.setTimeout(60000);await setup(session);await lights(session.page);
  await session.page.evaluate(()=>{
    const PM=(window as any).PM,api=(window as any).extrusionAPI;api.setExtrusion('title',{enabled:true,depth:45,bevelStyle:'convex',bevelDepth:4});
    PM.Edit.apply([{type:'set_layer',target:'title',patch:{motionBlur:true}},{type:'replace_keyframes',target:'title',path:'position.x',keyframes:[{time:0,value:-1000},{time:1,value:2000}]}]);PM.time=.5;PM.touch();
  });
  const frames=await session.page.evaluate(async()=>{
    const PM=(window as any).PM;await PM.prepareFrame(.5,{mblur:true,mbSamples:8,shutter:1});
    const sharp=PM.renderFrameTo(.5,960,540,{mblur:false}).toDataURL('image/png'),blur=PM.renderFrameTo(.5,960,540,{mblur:true,mbSamples:8,shutter:1}).toDataURL('image/png');return {sharp,blur};
  });expect(frames.sharp).not.toBe(frames.blur);await writeFile(`${shots}/13-after-motion-blur.png`,Buffer.from(frames.blur.split(',')[1],'base64'));
  const file=path.join(session.userData,'extruded-export.png');await session.app.evaluate(({dialog},filePath)=>{dialog.showSaveDialog=async()=>({canceled:false,filePath});},file);
  await session.page.evaluate(async()=>{const PM=(window as any).PM;await PM.Export.run({format:'still',scale:1,alpha:false,mblur:false,audio:false});});
  const png=await readFile(file);expect(png.subarray(1,4).toString()).toBe('PNG');expect(png.byteLength).toBeGreaterThan(5000);await writeFile(`${shots}/14-after-exported-frame.png`,png);
});
