import { createServer } from 'node:http';
import { chromium } from 'playwright';
import { expect,test } from './helpers/app';

test('agent builds a textured animated scene and undoes the entire run',async({session})=>{
  await session.openEditor();
  const p=await session.page.evaluate(async()=>{
    const PM=(window as any).PM,canvas=document.createElement('canvas');canvas.width=canvas.height=16;
    const ctx=canvas.getContext('2d')!;ctx.fillStyle='#EE3322';ctx.fillRect(0,0,16,16);
    const blob=await new Promise<Blob>(r=>canvas.toBlob(b=>r(b!)));
    const asset=await PM.assets.add(new File([blob],'paint.png',{type:'image/png'}),{silent:true});
    const before=JSON.stringify(PM.proj.layers),baseRevision=PM.proj.revision;
    const call=(tool:string,args={})=>PM.AgentHarness.test.handleLiveAgentTool({runId:'scene-e2e',callId:tool,tool,arguments:args,baseRevision});
    await call('get_project_state');const target='body';
    await call('edit_3d',{operation:'add_object',target,object:{id:'body',source:{primitive:'box',parameters:{width:1.4,depth:.6}},material:{p:{color:'#FFFFFF',metalness:0},maps:{color:asset.id}}}});
    await call('edit_3d',{operation:'add_object',target,object:{id:'orb',source:{primitive:'sphere'},p:{x:1220},material:{p:{color:'#3388FF'}}}});
    await call('edit_3d',{operation:'add_object',target,object:{id:'floor',source:{primitive:'plane',parameters:{width:6,height:6}},p:{y:660,rx:-90}}});
    await call('edit_3d',{operation:'add_light',target,light:{id:'fill',type:'point',p:{intensity:25,color:'#66BBFF',x:1360,y:140,z:-600}}});
    await call('apply_commands',{commands:[{type:'replace_keyframes',target:'body',path:'rotation.y',keyframes:[{time:0,value:0},{time:2,value:180}]}]});
    await call('edit_3d',{operation:'add_light',light:{id:'key',type:'sun'}});
    await call('edit_3d',{operation:'add_camera'});
    const state=JSON.parse((await call('get_3d_scene',{target})).content[0].text);
    const render=(t=0)=>PM.GL.renderToPixels(t,320,180,{transparent:true,mblur:false}),first=render(),animated=render(1);
    const frames=await call('render_frames',{times:[0,1],width:320});
    const saved=JSON.parse(PM.serialize()).proj.layers.find((l:any)=>l.id===target).p;
    const finish=await call('__finish_run',{commit:true});PM.hist.undo();
    return {scene:state.scene,paths:state.channels.map((p:any)=>p.path),lit:first.reduce((a:number,b:number)=>a+b,0),corner:first[3],
      center:[...first.slice((90*320+160)*4,(90*320+160)*4+4)],changed:JSON.stringify([...first])!==JSON.stringify([...animated]),
      frames:frames.content.filter((c:any)=>c.type==='image').length,saved,restored:JSON.stringify(PM.proj.layers)===before,finish:finish.ok};
  });
  expect(p.scene.objects).toHaveLength(3);expect(p.scene.lights).toHaveLength(2);
  expect(p.paths).toContain('m.color');expect(p.paths).toContain('camera.fov');expect(p.lit).toBeGreaterThan(10000);
  expect(p.center[0]).toBeGreaterThan(p.center[1]);expect(p.center[3]).toBe(255);
  expect(p).toMatchObject({corner:0,changed:true,frames:2,restored:true,finish:true});expect(p.saved['rotation.y'].kf).toHaveLength(2);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('portable export preserves imported glTF materials and matches the editor',async({session})=>{
  await session.openEditor();const exported=await session.page.evaluate(async()=>{
    const PM=(window as any).PM,api=PM.Kernel.api('scene-export-test'),position=new Float32Array([-1,-1,0,1,-1,0,0,1,0]);
    const bytes=new Uint8Array(position.buffer);let binary='';for(const b of bytes)binary+=String.fromCharCode(b);
    const source={asset:{version:'2.0'},scene:0,scenes:[{nodes:[0]}],nodes:[{mesh:0}],meshes:[{primitives:[{attributes:{POSITION:0},material:0}]}],
      materials:[{pbrMetallicRoughness:{baseColorFactor:[.1,.8,.2,1],metallicFactor:0,roughnessFactor:.5},doubleSided:true}],
      buffers:[{byteLength:bytes.length,uri:'data:application/octet-stream;base64,'+btoa(binary)}],bufferViews:[{buffer:0,byteOffset:0,byteLength:bytes.length}],
      accessors:[{bufferView:0,componentType:5126,count:3,type:'VEC3',min:[-1,-1,0],max:[1,1,0]}]};
    const asset=await PM.assets.add(new File([JSON.stringify(source)],'embedded.gltf',{type:'model/gltf+json'}),{silent:true});
    PM.proj.w=320;PM.proj.h=180;PM.setTime(0);
    api.scene3d.edit({operation:'create',scene:{objects:[{id:'model',source:{assetId:asset.id},useSourceMaterials:true}],lights:[{id:'sun',type:'sun'}],camera:{p:{x:160,y:90,z:-800}}}});
    PM.proj.w=320;PM.proj.h=180;PM.touch();PM.GL.resize(320,180);
    const result=await PM.Export.buildWeb();PM.GL.render(0,{exporting:true,mblur:false});
    const pixels=new Uint8Array(320*180*4),gl=PM.GL.gl;gl.readPixels(0,0,320,180,gl.RGBA,gl.UNSIGNED_BYTE,pixels);
    return {files:[...result.files].map(([name,data]:any)=>[name,[...data]]),pixels:[...pixels]};
  });
  const files=new Map<string,Buffer>(exported.files.map(([name,data]:any)=>[name,Buffer.from(data)]));
  const server=createServer((req,res)=>{const name=req.url==='/'?'index.html':req.url!.slice(1),data=files.get(name);res.statusCode=data?200:404;
    res.setHeader('Content-Type',name.endsWith('.js')?'text/javascript':name.endsWith('.json')?'application/json':name.endsWith('.gltf')?'model/gltf+json':'text/html');res.end(data||'Not found');});
  await new Promise<void>(r=>server.listen(0,'127.0.0.1',r));const browser=await chromium.launch({headless:true});
  try{const page=await browser.newPage();await page.goto(`http://127.0.0.1:${(server.address() as any).port}`);
    const actual=await page.evaluate(async()=>{const {createPlayer}=await import(/* @vite-ignore */ location.origin+'/player.js');
      const canvas=document.createElement('canvas');canvas.style.width='320px';canvas.style.height='180px';document.body.append(canvas);
      const player=await createPlayer({canvas,scene:'./scene.json',audio:false});await player.seek(0);
      const gl=canvas.getContext('webgl2')!,pixels=new Uint8Array(320*180*4);gl.readPixels(0,0,320,180,gl.RGBA,gl.UNSIGNED_BYTE,pixels);player.destroy();return [...pixels];});
    expect(actual.reduce((sum,v,i)=>sum+Math.abs(v-exported.pixels[i]!),0)/actual.length).toBeLessThan(1);
    expect(exported.pixels[(90*320+160)*4+1]).toBeGreaterThan(exported.pixels[(90*320+160)*4]);
  }finally{await browser.close();await new Promise<void>(r=>server.close(()=>r()));}
  expect(session.diagnostics.pageErrors).toEqual([]);
});


test('OBJ material bundles retain UV textures as a durable GLB',async({session})=>{
  await session.openEditor();
  const proof=await session.page.evaluate(async()=>{
    const PM=(window as any).PM,api=PM.Kernel.api('material-import-test');
    const canvas=document.createElement('canvas');canvas.width=canvas.height=8;
    const ctx=canvas.getContext('2d')!;ctx.fillStyle='#FF1100';ctx.fillRect(0,0,8,8);
    const blob=await new Promise<Blob>(r=>canvas.toBlob(b=>r(b!)));
    const source='mtllib model.mtl\nv -1 -1 0\nv 1 -1 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt .5 1\nusemtl Paint\nf 1/1 2/2 3/3\n';
    const files=[new File([source],'model.obj',{type:'model/obj'}),new File(['newmtl Paint\nKd 1 1 1\nmap_Kd paint.png\n'],'model.mtl'),new File([blob],'paint.png',{type:'image/png'})];
    const packed=await api.scene3d.prepareImport(files);
    const imported=await PM.cmd('3d.import-obj',packed);
    const layer=PM.proj.layers.find((l:any)=>l.d?.definition==='powermove.3d.object'),object=layer.d.data.object,asset=PM.assets.get(object.source.assetId);
    let textured=false;asset.object3d.traverse((o:any)=>{if(o.material?.map)textured=true;});
    const bytes=PM.GL.renderToPixels(0,320,180,{transparent:true,mblur:false}),center=[...bytes.slice((90*320+160)*4,(90*320+160)*4+4)];
    let missingRejected=false;try{await api.scene3d.prepareImport(files.slice(0,2));}catch(e){missingRejected=String(e).includes('missing texture');}
    return {name:packed.name,format:asset.format,textured,sourceMaterial:object.useSourceMaterials,center,missingRejected,imported:imported.ok,
      stored:(await PM.MediaStore.get(PM.proj.assets[asset.id])).size};
  });
  expect(proof).toMatchObject({name:'model.glb',format:'glb',textured:true,sourceMaterial:true,missingRejected:true,imported:true});
  expect(proof.center[0]).toBeGreaterThan(proof.center[1]);expect(proof.center[3]).toBe(255);expect(proof.stored).toBeGreaterThan(100);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('legacy models convert to editable scenes and undo back to their original renderer',async({session})=>{
  await session.openEditor();const result=await session.page.evaluate(async()=>{
    const PM=(window as any).PM;
    const asset=await PM.assets.add(new File(['v -1 -1 0\nv 1 -1 0\nv 0 1 0\nf 1 2 3\n'],'old.obj',{type:'model/obj'}),{silent:true});
    PM.Edit.apply({type:'add_layer',layerType:'extension',content:{definition:'powermove.3d.obj-model',data:{assetId:asset.id}}});
    const layer=PM.proj.layers[0],old=JSON.stringify(layer.d);
    PM.Edit.apply({type:'replace_keyframes',target:layer.id,path:'x.yaw',keyframes:[{time:0,value:0},{time:2,value:90}]});
    const original=JSON.stringify(PM.L(layer.id).d),before=PM.Export.snapshot(0,160),count=PM.hist.list().length;
    const converted=PM.cmd('3d.convert-layer',layer.id),upgraded=PM.L(layer.id);
    const proof={ok:converted.ok,definition:upgraded.d.definition,objects:PM.proj.layers.filter((l:any)=>l.d?.definition==='powermove.3d.object').length,cameraKeys:PM.proj.layers.find((l:any)=>l.d?.definition==='powermove.3d.camera').p['position.x'].kf.length,
      background:PM.proj.layers.find((l:any)=>l.d?.definition==='powermove.3d.camera').d.data.environment.background,undoSteps:PM.hist.list().length-count,rendered:PM.Export.snapshot(0,160).length>1000};
    PM.hist.undo();return {...proof,restored:JSON.stringify(PM.L(layer.id).d)===original,pixelsRestored:PM.Export.snapshot(0,160)===before};
  });
  expect(result).toMatchObject({ok:true,definition:'powermove.3d.object',objects:1,background:'#0C0D12',undoSteps:1,rendered:true,restored:true,pixelsRestored:true});
  expect(result.cameraKeys).toBeGreaterThan(30);expect(session.diagnostics.pageErrors).toEqual([]);
});

test('normal maps and shadow lights change actual GPU output',async({session})=>{
  await session.openEditor();const proof=await session.page.evaluate(async()=>{
    const PM=(window as any).PM,api=PM.Kernel.api('shading-proof'),canvas=document.createElement('canvas');canvas.width=canvas.height=8;
    const ctx=canvas.getContext('2d')!;ctx.fillStyle='rgb(215,128,215)';ctx.fillRect(0,0,8,8);
    const blob=await new Promise<Blob>(r=>canvas.toBlob(b=>r(b!))),asset=await PM.assets.add(new File([blob],'normals.png',{type:'image/png'}),{silent:true});
    api.scene3d.edit({operation:'create',scene:{objects:[{id:'box',source:{primitive:'box'},material:{p:{color:'#FFFFFF',metalness:0}}},
      {id:'floor',source:{primitive:'plane',parameters:{width:6,height:6}},p:{y:660,rx:-90},material:{p:{color:'#FFFFFF'}}}],
      lights:[{id:'key',type:'sun',p:{x:360,y:-460,z:-800}}],environment:{p:{ambient:.15}}}});
    const target=PM.sel.layers[0],render=()=>PM.GL.renderToPixels(0,320,180,{transparent:true,mblur:false}),baseline=render();
    api.scene3d.edit({operation:'set_environment',target,patch:{shadows:false}});const noShadow=render();
    api.scene3d.edit({operation:'set_environment',target,patch:{shadows:true}});
    api.scene3d.edit({operation:'update_object',target,id:PM.proj.layers.find((l:any)=>l.d?.data?.object?.source?.primitive==='box').id,patch:{material:{maps:{normal:asset.id}}}});const mapped=render();
    const difference=(a:any,b:any)=>a.reduce((sum:number,v:number,i:number)=>sum+Math.abs(v-b[i]),0);
    return {shadowDifference:difference(baseline,noShadow),normalDifference:difference(baseline,mapped),corners:[baseline[3],mapped[3]]};
  });
  expect(proof.shadowDifference).toBeGreaterThan(100);expect(proof.normalDifference).toBeGreaterThan(100);expect(proof.corners).toEqual([0,0]);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
