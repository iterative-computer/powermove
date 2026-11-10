import { readFile, writeFile } from 'node:fs/promises';
import {execFileSync} from 'node:child_process';
import path from 'node:path';
import { test, expect,repoRoot } from './helpers/app';
import { screenGlb } from '../src/renderer/src/core/scene3d/__fixtures__/screen-glb';

const shots='/tmp/pm-figuate-feedback/shots/comp-material-engine';
test('named screen material baseline',async({session})=>{
  await session.openEditor();
  const png=await session.page.evaluate(async bytes=>{
    const PM=(window as any).PM,api=PM.Kernel.api('material-proof');
    api.scene3d.edit({operation:'add_camera',camera:{p:{x:0,y:0,z:3}}});
    const imported=await PM.cmd('3d.import-obj',new File([new Uint8Array(bytes)],'screen.glb',{type:'model/gltf-binary'}));
    if(!imported.ok)throw new Error(imported.message);
    return PM.renderFrameTo(0,640,360,{mblur:false}).toDataURL('image/png');
  },Array.from(screenGlb()));
  await writeFile(`${shots}/01-before-original-screen.png`,Buffer.from(png.split(',')[1],'base64'));
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('composition pixels animate on the original Screen material in preview/export and survive a file reopen',async({session})=>{
  await session.openEditor();const {page}=session;
  const setup=await page.evaluate(async bytes=>{
    const PM=(window as any).PM,api=PM.Kernel.api('material-proof'),main=PM.proj.compId;
    const source=PM.Comps.create({name:'Screen animation',w:128,h:128,fps:24,dur:3},{open:true});
    const add=(id:string,color:string,y:number)=>PM.Edit.apply({type:'add_layer',id,layerType:'solid',content:{w:128,h:64,color},
      properties:{'position.x':64,'position.y':y,'anchor.x':64,'anchor.y':32}});
    add('upper','#FF0000',32);add('lower','#0000FF',96);
    PM.Edit.apply(['upper','lower'].flatMap((id,index)=>[
      {type:'set_property',target:id,path:'position.x',value:64},
      {type:'set_property',target:id,path:'position.y',value:index?96:32}]));
    PM.Edit.apply({type:'replace_keyframes',target:'upper',path:'c.color',keyframes:[{time:0,value:'#FF0000',hold:true},{time:1,value:'#00FF00',hold:true}]});
    PM.Comps.open(main);api.scene3d.edit({operation:'add_camera',camera:{p:{x:0,y:0,z:3}}});
    const imported=await PM.cmd('3d.import-obj',new File([new Uint8Array(bytes)],'screen.glb',{type:'model/gltf-binary'}));
    if(!imported.ok)throw new Error(imported.message);
    const model=PM.proj.layers.find((l:any)=>l.d?.data?.object),id=model.id;
    PM.Edit.apply({type:'set_layer',target:id,patch:{from:.5}});
    const materials=api.scene3d.listMaterials(id),valid=api.scene3d.listValidCompositions(id);
    const applied=api.scene3d.setMaterialMap(id,0,'emissive',{compositionId:source});
    if(!applied.ok)throw new Error(applied.message);
    const color=api.scene3d.setMaterialMap(id,0,'color',{compositionId:source});if(!color.ok)throw new Error(color.message);
    PM.setTime(.5);
    return {id,source,main,materials,valid};
  },Array.from(screenGlb()));
  expect(setup.materials).toMatchObject([{index:0,name:'Screen',slots:expect.arrayContaining(['color','emissive'])}]);
  expect(setup.valid.map((c:any)=>c.id)).toContain(setup.source);expect(setup.valid.map((c:any)=>c.id)).not.toContain(setup.main);
  const capture=async(time:number,name?:string)=>{
    const result=await page.evaluate(time=>{
      const PM=(window as any).PM,canvas=PM.renderFrameTo(time,640,360,{mblur:false});
      const context=canvas.getContext('2d'),sample=(x:number,y:number)=>[...context.getImageData(x,y,1,1).data];
      return {png:canvas.toDataURL('image/png'),top:sample(320,130),bottom:sample(320,230)};
    },time);
    if(name)await writeFile(`${shots}/${name}.png`,Buffer.from(result.png.split(',')[1],'base64'));
    return result;
  };
  const first=await capture(.5,'02-after-comp-red-top-blue-bottom');
  expect(first.top[0]).toBeGreaterThan(first.top[1]+30);expect(first.bottom[2]).toBeGreaterThan(first.bottom[0]+30);
  const next=await capture(1.5,'03-after-comp-animated-green-top');
  expect(next.top[1]).toBeGreaterThan(next.top[0]+30);expect(next.bottom[2]).toBeGreaterThan(next.bottom[0]+30);
  const proof=await page.evaluate(({id,source})=>{
    const PM=(window as any).PM,api=PM.Kernel.api('material-proof');
    const original=PM.GL.renderMaterialComposition;let captures=0;
    PM.GL.renderMaterialComposition=(...args:any[])=>{captures++;return original(...args);};
    PM.Comps.rename(source,'Renamed screen');
    for(const t of [.5,.501,.502])PM.GL.renderToPixels(t,320,180,{transparent:true,mblur:false});
    PM.GL.renderMaterialComposition=original;
    const clear=api.scene3d.setMaterialMap(id,0,'emissive',null);
    const cleared=!PM.L(id).d.data.object.sourceMaterials[0]?.emissive;PM.hist.undo();
    const undo=PM.L(id).d.data.object.sourceMaterials[0].emissive.compositionId===source;
    PM.hist.redo();const redo=!PM.L(id).d.data.object.sourceMaterials[0]?.emissive;PM.hist.undo();
    const rejected=api.scene3d.setMaterialMap(id,0,'color',{compositionId:PM.proj.compId});
    PM.GL.render(.5,{mblur:false,previewReuse:false});
    const gl=PM.GL.gl,pixel=new Uint8Array(4),w=gl.drawingBufferWidth,h=gl.drawingBufferHeight;
    gl.readPixels(Math.round(w*.5),Math.round(h*(1-130/360)),1,1,gl.RGBA,gl.UNSIGNED_BYTE,pixel);
    return {captures,clear:clear.ok,cleared,undo,redo,rejected:!rejected.ok,preview:[...pixel]};
  },setup);
  expect(proof).toMatchObject({captures:1,clear:true,cleared:true,undo:true,redo:true,rejected:true});
  expect(proof.preview[0]).toBeGreaterThan(proof.preview[1]+30);
  const filePath=`${session.userData}/composition-material.pmv`;
  await session.app.evaluate(({dialog},filePath)=>{
    dialog.showSaveDialog=async()=>({canceled:false,filePath});dialog.showOpenDialog=async()=>({canceled:false,filePaths:[filePath]});
  },filePath);
  await page.evaluate(async()=>{const PM=(window as any).PM;await PM.saveProject();await PM.openProject();});
  await expect.poll(()=>page.evaluate(({id,source})=>{
    const PM=(window as any).PM;
    return PM.L(id)?.d?.data?.object?.sourceMaterials?.[0]?.emissive?.compositionId===source
      && !!PM.assets.get(PM.L(id).d.data.object.source.assetId)?.object3d;
  },setup)).toBe(true);
  const reopened=await capture(1.5,'04-after-save-reopen');expect(reopened.top[1]).toBeGreaterThan(reopened.top[0]+30);
  await page.evaluate(source=>(window as any).PM.Comps.remove(source),setup.source);
  const deleted=await capture(1.5,'05-after-source-comp-deleted');
  expect(Math.abs(deleted.top[0]-deleted.top[1])).toBeLessThan(3);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('a material comp prepares video at the model source time and can switch to an image',async({session})=>{
  await session.openEditor();
  const file=path.join(session.userData,'screen-video.webm');
  const frames=Buffer.concat(Array.from({length:20},(_,i)=>{
    const frame=Buffer.alloc(64*64*3);for(let p=0;p<frame.length;p+=3)frame[p+(i<10?0:1)]=255;return frame;
  }));
  execFileSync(path.join(repoRoot,'node_modules/ffmpeg-static/ffmpeg'),[
    '-v','error','-f','rawvideo','-pix_fmt','rgb24','-s','64x64','-r','10','-i','pipe:0',
    '-c:v','libvpx-vp9','-lossless','1','-g','1',file],{input:frames});
  const result=await session.page.evaluate(async({video,model})=>{
    const PM=(window as any).PM,api=PM.Kernel.api('material-video'),main=PM.proj.compId;
    const source=PM.Comps.create({name:'Video screen',w:64,h:64,fps:10,dur:2},{open:true});
    const media=await PM.assets.add(new File([new Uint8Array(video)],'screen-video.webm',{type:'video/webm'}),{silent:true});
    PM.Edit.apply({type:'add_layer',id:'video-screen',layerType:'video',content:{asset:media.id,w:64,h:64,fit:'stretch'}});
    PM.Edit.apply(['position.x','position.y'].map(path=>({type:'set_property',target:'video-screen',path,value:32})));
    PM.Edit.apply(['anchor.x','anchor.y'].map(path=>({type:'set_property',target:'video-screen',path,value:0})));
    PM.Comps.open(main);api.scene3d.edit({operation:'add_camera',camera:{p:{x:0,y:0,z:3}}});
    await PM.cmd('3d.import-obj',new File([new Uint8Array(model)],'screen.glb',{type:'model/gltf-binary'}));
    const layer=PM.proj.layers.find((l:any)=>l.d?.data?.object),id=layer.id;
    PM.Edit.apply([{type:'set_layer',target:id,patch:{from:.5}},{type:'set_content',target:id,patch:{speed:2}}]);
    for(const slot of ['color','emissive']){const applied=api.scene3d.setMaterialMap(id,0,slot,{compositionId:source});if(!applied.ok)throw new Error(applied.message);}
    const capture=async(time:number)=>{
      await PM.prepareFrame(time,{width:320,height:180,mblur:false});
      const pixels=PM.GL.renderToPixels(time,320,180,{transparent:true,mblur:false});
      return [...pixels.slice((90*320+160)*4,(90*320+160)*4+4)];
    };
    const red=await capture(.5),green=await capture(1);
    const image=document.createElement('canvas');image.width=image.height=4;const ctx=image.getContext('2d')!;ctx.fillStyle='#0000FF';ctx.fillRect(0,0,4,4);
    const blob=await new Promise<Blob>(resolve=>image.toBlob(blob=>resolve(blob!))),asset=await PM.assets.add(new File([blob],'blue.png',{type:'image/png'}),{silent:true});
    for(const slot of ['color','emissive'])api.scene3d.setMaterialMap(id,0,slot,asset.id);
    const blue=await capture(1);
    return {red,green,blue,source,stored:PM.L(id).d.data.object.sourceMaterials[0]};
  },{video:[...await readFile(file)],model:[...screenGlb()]});
  expect(result.red[0],JSON.stringify(result)).toBeGreaterThan(result.red[1]+30);
  expect(result.green[1],JSON.stringify(result)).toBeGreaterThan(result.green[0]+30);
  expect(result.blue[2],JSON.stringify(result)).toBeGreaterThan(result.blue[0]+30);
  expect(result.red[3]).toBe(255);expect(result.green[3]).toBe(255);
  expect(typeof result.stored.color).toBe('string');expect(session.diagnostics.pageErrors).toEqual([]);
});
