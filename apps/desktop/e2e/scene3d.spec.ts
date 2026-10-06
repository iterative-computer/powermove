import { createServer } from 'node:http';
import { chromium } from 'playwright';
import { expect,test } from './helpers/app';

test('3D groups rotate with one gizmo and preserve their members and Undo',async({session})=>{
  await session.openEditor();const {page}=session;
  const initial=await page.evaluate(()=>{
    const PM=(window as any).PM,api=PM.Kernel.api('group-proof');
    api.scene3d.edit({operation:'add_camera',camera:{p:{x:0,y:0,z:6}}});
    for(const [id,x,color] of [['left',-.7,'#FF7733'],['right',.7,'#3388FF']])api.scene3d.edit({operation:'add_object',object:{id,source:{primitive:'box'},p:{x},material:{p:{color}}}});
    const render=()=>[...PM.GL.renderToPixels(0,160,90,{transparent:true,mblur:false})],pixels=render();
    PM.Edit.apply({type:'group_layers',targets:['left','right'],name:'Models'});
    const group=PM.proj.layers.find((l:any)=>l.type==='group');PM.selectLayers([group.id]);PM.invalidate();
    return {id:group.id,pixels,groupedPixels:render(),members:['left','right'].map(id=>JSON.stringify(PM.L(id).p)),count:PM.hist.list().length};
  });
  expect(initial.groupedPixels).toEqual(initial.pixels);
  await expect(page.locator('#composition-3d-controls')).toBeVisible();
  await page.getByRole('radio',{name:'Rotate',exact:true}).click();
  const handle=await page.evaluate(()=>{
    const PM=(window as any).PM,viewer=PM.Kernel.services.get('viewer'),canvas=document.querySelector('#gl')!.getBoundingClientRect(),cx=canvas.left+canvas.width/2,cy=canvas.top+canvas.height/2;
    for(let y=cy-3;y<cy+8;y+=2)for(let x=cx+35;x<cx+130;x+=2)
      if(viewer.sceneGizmo.hover(x,y)&&(document.querySelector('[data-scene-gizmo]') as HTMLElement).dataset.gizmoAxis==='Z')return {x,y,cx,cy};
    throw new Error('Missing group rotation gizmo');
  });
  await page.keyboard.down('Control');await page.mouse.move(handle.x,handle.y);await page.mouse.down();await page.mouse.move(handle.cx,handle.cy+90,{steps:10});await page.mouse.up();await page.keyboard.up('Control');
  const rotated=await page.evaluate(id=>{const PM=(window as any).PM;return {rotation:PM.L(id).p.rotation.v,count:PM.hist.list().length,members:['left','right'].map(id=>JSON.stringify(PM.L(id).p)),pixels:[...PM.GL.renderToPixels(0,160,90,{transparent:true,mblur:false})]};},initial.id);
  expect(Math.abs(rotated.rotation)).toBeGreaterThan(45);expect(rotated.rotation/5).toBeCloseTo(Math.round(rotated.rotation/5));
  expect(rotated.members).toEqual(initial.members);expect(rotated.count).toBe(initial.count+1);expect(rotated.pixels).not.toEqual(initial.pixels);
  await page.evaluate(()=>(window as any).PM.hist.undo());
  expect(await page.evaluate(()=>[...(window as any).PM.GL.renderToPixels(0,160,90,{transparent:true,mblur:false})])).toEqual(initial.pixels);
  for(const theme of ['dark','light']){
    await page.evaluate(theme=>{const PM=(window as any).PM;PM.theme.apply(theme);PM.invalidate();},theme);
    await expect.poll(()=>page.evaluate(()=>{
      const gl=(window as any).PM.GL.gl,pixels=new Uint8Array(400*200*4);gl.bindFramebuffer(gl.FRAMEBUFFER,null);
      gl.readPixels(Math.max(0,Math.floor(gl.drawingBufferWidth/2)-200),Math.max(0,Math.floor(gl.drawingBufferHeight/2)-100),400,200,gl.RGBA,gl.UNSIGNED_BYTE,pixels);
      return pixels.reduce((sum,v,i)=>sum+(i%4===3?0:v),0);
    })).toBeGreaterThan(100000);
    await page.evaluate(()=>new Promise<void>(resolve=>requestAnimationFrame(()=>requestAnimationFrame(()=>resolve()))));
    // Flush the retained WebGL surface into Chromium's compositor before capture.
    await page.locator('#gl').evaluate((canvas:HTMLCanvasElement)=>{canvas.toDataURL();});
    await page.screenshot({path:`/tmp/powermove-3d-group-${theme}.png`});
  }
  await page.evaluate(id=>{
    const PM=(window as any).PM;PM.Edit.apply([{type:'set_layer',target:id,patch:{threeD:false}},
      ...['x','y'].flatMap((axis,i)=>['position','anchor'].map(key=>({type:'set_property',target:id,path:`${key}.${axis}`,value:i?540:960})))]);PM.invalidate();
  },initial.id);
  await expect.poll(()=>page.evaluate(({x,y})=>(window as any).PM.Kernel.services.get('viewer').sceneGizmo.hover(x,y),handle)).toBe(true);
  await page.mouse.move(handle.x,handle.y);await page.mouse.down();await page.mouse.move(handle.cx,handle.cy+90,{steps:8});await page.mouse.up();
  const upgraded=await page.evaluate(id=>{const l=(window as any).PM.L(id);return {threeD:l.threeD,anchor:[l.p['anchor.x'].v,l.p['anchor.y'].v]};},initial.id);
  expect(upgraded.threeD).toBe(true);upgraded.anchor.forEach(v=>expect(v).toBeCloseTo(0));
  await page.evaluate(()=>(window as any).PM.hist.undo());
  expect(await page.evaluate(id=>(window as any).PM.L(id).p['anchor.x'].v,initial.id)).toBe(960);
  expect(await page.evaluate(()=>[...(window as any).PM.GL.renderToPixels(0,160,90,{transparent:true,mblur:false})])).toEqual(initial.pixels);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('Blender navigation: MMB orbit, Shift+MMB pan, numpad views and framing preserve the render camera, project and export',async({session})=>{
  await session.openEditor();const {page}=session;
  const initial=await page.evaluate(()=>{
    const PM=(window as any).PM,api=PM.Kernel.api('navigation-proof');
    api.scene3d.edit({operation:'add_camera',camera:{p:{x:0,y:0,z:6}}});
    api.scene3d.edit({operation:'add_object',object:{id:'model',source:{primitive:'box',parameters:{width:1.6,depth:.6}},p:{x:.7},material:{p:{color:'#FF7733'}}}});
    PM.proj.layers.find((l:any)=>l.d?.definition==='powermove.3d.camera').lock=true;PM.autokey=true;PM.selectLayers(['model']);PM.invalidate();
    return {layers:JSON.stringify(PM.proj.layers),count:PM.hist.list().length,
      pixels:[...PM.GL.renderToPixels(0,160,90,{transparent:true,mblur:false})]};
  });
  const sample=()=>page.evaluate(()=>{
    const PM=(window as any).PM;PM.GL.render(0,{mblur:false,previewReuse:false});
    const gl=PM.GL.gl,pixels=new Uint8Array(gl.drawingBufferWidth*gl.drawingBufferHeight*4);
    gl.readPixels(0,0,gl.drawingBufferWidth,gl.drawingBufferHeight,gl.RGBA,gl.UNSIGNED_BYTE,pixels);
    let hash=2166136261;for(const value of pixels)hash=Math.imul(hash^value,16777619);return hash;
  });
  const label=page.locator('.vp-label span').first();
  await expect(label).toHaveText('Camera Perspective');
  await expect(page.getByRole('img',{name:/Navigation gizmo/})).toBeVisible();
  const original=await sample();
  const box=await page.locator('#stage').boundingBox(),point={x:box!.x+box!.width*.6,y:box!.y+box!.height*.6};
  const drag=async(dx:number,dy:number,shift=false)=>{
    await page.mouse.move(point.x,point.y);if(shift)await page.keyboard.down('Shift');
    await page.mouse.down({button:'middle'});await page.mouse.move(point.x+dx,point.y+dy,{steps:8});await page.mouse.up({button:'middle'});
    if(shift)await page.keyboard.up('Shift');
  };
  await drag(65,-30);await expect(label).toHaveText('User Perspective');
  await expect.poll(sample).not.toBe(original);const orbited=await sample();
  await page.mouse.move(point.x,point.y);await page.mouse.down({button:'middle'});await page.mouse.move(point.x+45,point.y,{steps:5});await page.keyboard.press('Escape');await page.mouse.up({button:'middle'});
  expect(await sample()).toBe(orbited);
  await drag(40,25,true);await expect.poll(sample).not.toBe(orbited);const panned=await sample();
  await page.mouse.move(point.x,point.y);await page.keyboard.press('7');await expect(label).toHaveText('Top Orthographic');
  await expect.poll(sample).not.toBe(panned);
  await page.keyboard.press('5');await expect(label).toHaveText('Top Perspective');
  await page.keyboard.press('1');await expect(label).toHaveText('Front Orthographic');
  await page.keyboard.press('Control+3');await expect(label).toHaveText('Left Orthographic');
  const beforeFrame=await sample();
  // Playwright's NumpadDecimal has no NumLock; send the period a numpad actually produces.
  await page.evaluate(()=>{for(const type of ['keydown','keyup'])document.body.dispatchEvent(new KeyboardEvent(type,{key:'.',code:'NumpadDecimal',bubbles:true}));});await expect.poll(sample).not.toBe(beforeFrame);
  await page.keyboard.press('Home');
  await page.getByRole('radio',{name:'Move',exact:true}).click();
  await page.keyboard.press('1');
  const aligned=await page.evaluate(()=>{
    const PM=(window as any).PM,b=PM.GL.bounds(PM.L('model'),0),viewer=PM.Kernel.services.get('viewer'),canvas=document.querySelector('#gl')!.getBoundingClientRect();
    const picked=PM.GL.pick((b.x0+b.x1)/2,(b.y0+b.y1)/2,0)?.id;
    let handle=null;
    for(let y=canvas.top+canvas.height*.2;y<canvas.top+canvas.height*.8&&!handle;y+=4)
      for(let x=canvas.left+canvas.width*.2;x<canvas.left+canvas.width*.8;x+=4)
        if(viewer.sceneGizmo.hover(x,y)&&(document.querySelector('[data-scene-gizmo]') as HTMLElement).dataset.gizmoAxis==='X'){handle={x,y};break;}
    return {picked,handle};
  });
  expect(aligned.picked).toBe('model');expect(aligned.handle).not.toBeNull();
  // Editing in a user view still edits the real object and gets one Undo.
  await page.mouse.move(aligned.handle!.x,aligned.handle!.y);await page.mouse.down();await page.mouse.move(aligned.handle!.x+35,aligned.handle!.y,{steps:8});await page.mouse.up();
  expect(await page.evaluate(()=>(window as any).PM.hist.list().length)).toBe(initial.count+1);
  await page.evaluate(()=>(window as any).PM.hist.undo());
  const unchanged=await page.evaluate(()=>{const PM=(window as any).PM;return {layers:JSON.stringify(PM.proj.layers),pixels:[...PM.GL.renderToPixels(0,160,90,{transparent:true,mblur:false})]};});
  expect(unchanged.layers).toBe(initial.layers);expect(unchanged.pixels).toEqual(initial.pixels);
  await page.mouse.move(point.x,point.y);await page.keyboard.press('0');await expect(label).toHaveText('Camera Perspective');
  await expect.poll(sample).toBe(original);
  await page.mouse.wheel(0,80);
  expect(await page.evaluate(()=>(window as any).PM.Kernel.api('navigation-proof').scene3d.getView())).toBe('camera');
  await page.keyboard.press('0');await expect(label).toHaveText('Front Orthographic');
  await page.getByRole('button',{name:/Toggle camera view/}).click();await expect(label).toHaveText('Camera Perspective');
  expect(session.diagnostics.pageErrors).toEqual([]);
});

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
    await call('edit_3d',{operation:'add_object',target,object:{id:'orb',source:{primitive:'sphere'},p:{x:1.3},material:{p:{color:'#3388FF'}}}});
    await call('edit_3d',{operation:'add_object',target,object:{id:'floor',source:{primitive:'plane',parameters:{width:6,height:6}},p:{y:-.6,rx:-90}}});
    await call('edit_3d',{operation:'add_light',target,light:{id:'fill',type:'point',p:{intensity:25,color:'#66BBFF',x:2,y:2,z:3}}});
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

test('selecting an imported model shows composition gizmos; drag, cancel and undo edit its layer',async({session})=>{
  await session.openEditor();const {page}=session;
  const initial=await page.evaluate(async()=>{
    const PM=(window as any).PM,api=PM.Kernel.api('gizmo-proof');PM.setTime(0);
    api.scene3d.edit({operation:'add_camera',camera:{p:{x:0,y:0,z:6}}});
    const imported=await PM.cmd('3d.import-obj',new File(['v -1 -1 0\nv 1 -1 0\nv 0 1 0\nf 1 2 3\n'],'triangle.obj',{type:'model/obj'}));
    const target=imported.data.result.id;PM.selectLayers([target]);PM.invalidate();return {target,count:PM.hist.list().length};
  });
  await expect(page.locator('#composition-3d-controls')).toBeVisible();
  await page.getByRole('radio',{name:'Move',exact:true}).click();
  const findHandle=(axis:string)=>page.evaluate((axis)=>{
    const PM=(window as any).PM,viewer=PM.Kernel.services.get('viewer'),stage=document.querySelector('#stage')!,rect=stage.getBoundingClientRect();
    const canvas=document.querySelector('#gl')!.getBoundingClientRect(),cx=canvas.left+canvas.width/2,cy=canvas.top+canvas.height/2;
    for(let y=cy;y<Math.min(cy+100,rect.bottom-5);y+=3)for(let x=cx+28;x<Math.min(cx+150,rect.right-5);x+=3)
      if(viewer.sceneGizmo.hover(x,y) && (document.querySelector('[data-scene-gizmo]') as HTMLElement)?.dataset.gizmoAxis===axis)return {x,y,cx,cy};
    throw new Error(`No ${axis} gizmo handle at model origin`);
  },axis);
  const handle=await findHandle('X');
  const position=()=>page.evaluate(id=>(window as any).PM.L(id).p['position.x'].v,initial.target);
  await page.mouse.move(handle.x,handle.y);await page.mouse.down();await page.mouse.move(handle.x+40,handle.y,{steps:8});await page.mouse.up();
  await expect.poll(position).not.toBe(0);
  expect(await page.evaluate(()=>(window as any).PM.hist.list().length)).toBe(initial.count+1);
  await page.evaluate(()=>(window as any).PM.hist.undo());expect(await position()).toBe(0);
  await page.mouse.move(handle.x,handle.y);await page.mouse.down();await page.mouse.move(handle.x+50,handle.y,{steps:5});await page.keyboard.press('Escape');await page.mouse.up();expect(await position()).toBe(0);
  await page.getByRole('radio',{name:'Rotate',exact:true}).click();
  const rotate=await findHandle('Z');await page.mouse.move(rotate.x,rotate.y);await page.mouse.down();await page.mouse.move(rotate.cx,rotate.cy+80,{steps:8});await page.mouse.up();
  expect(Math.abs(await page.evaluate(id=>(window as any).PM.L(id).p.rotation.v,initial.target))).toBeGreaterThan(10);
  await page.evaluate(()=>(window as any).PM.hist.undo());
  await page.getByRole('radio',{name:'Scale',exact:true}).click();
  const scale=await findHandle('X');await page.mouse.move(scale.x,scale.y);await page.mouse.down();await page.mouse.move(scale.x+35,scale.y,{steps:8});await page.mouse.up();
  expect(await page.evaluate(id=>(window as any).PM.L(id).p['scale.x'].v,initial.target)).toBeGreaterThan(100);
  await page.evaluate(()=>(window as any).PM.hist.undo());
  expect(await page.evaluate(id=>(window as any).PM.L(id).p['scale.x'].v,initial.target)).toBe(100);
  await page.getByRole('radio',{name:'Move',exact:true}).click();
  await page.keyboard.down('Control');await page.mouse.move(handle.x,handle.y);await page.mouse.down();await page.mouse.move(handle.x+90,handle.y,{steps:8});await page.mouse.up();await page.keyboard.up('Control');
  expect(await position()).not.toBe(0);expect(await position()).toBeCloseTo(Math.round(await position()),6);await page.evaluate(()=>(window as any).PM.hist.undo());
  await page.evaluate(()=>{const PM=(window as any).PM;PM.Kernel.services.get('viewer').setZoom(1);PM.invalidate();});
  const zoomed=await findHandle('X');await page.mouse.move(zoomed.x,zoomed.y);await page.mouse.down();await page.mouse.move(zoomed.x+30,zoomed.y,{steps:5});await page.mouse.up();expect(await position()).not.toBe(0);await page.evaluate(()=>(window as any).PM.hist.undo());
  await page.evaluate(()=>(window as any).PM.Kernel.services.get('viewer').returnToComposition());

  await page.evaluate(()=>(window as any).PM.Kernel.api('gizmo-state').scene3d.viewport.run('orientation','local'));
  expect(await page.evaluate(()=>(window as any).PM.Kernel.api('gizmo-state').scene3d.viewport.state().orientation)).toBe('local');
  await expect(page.getByRole('button',{name:'Transform orientation: Local'})).toBeVisible();
  await page.evaluate(()=>(window as any).PM.Kernel.api('gizmo-state').scene3d.viewport.run('orientation','global'));
  expect(await page.evaluate(()=>(window as any).PM.GL.renderToPixels(0,160,90,{transparent:true})[3])).toBe(0);
  const repeated=await page.evaluate(()=>{
    const PM=(window as any).PM;return [0,1,2].map(()=>PM.GL.renderToPixels(0,160,90,{transparent:true,mblur:false})[(45*160+80)*4+3]);
  });expect(repeated).toEqual([255,255,255]);
  for(const theme of ['dark','light']){
    await page.evaluate(theme=>(window as any).PM.theme.apply(theme),theme);
    await expect.poll(()=>page.evaluate(()=>{
      const PM=(window as any).PM,gl=PM.GL.gl,pixel=new Uint8Array(4);
      gl.bindFramebuffer(gl.FRAMEBUFFER,null);gl.readPixels(Math.floor(PM.GL.canvas.width/2),Math.floor(PM.GL.canvas.height/2),1,1,gl.RGBA,gl.UNSIGNED_BYTE,pixel);
      return pixel[0]+pixel[1]+pixel[2];
    })).toBeGreaterThan(100);
    await page.screenshot({path:`/tmp/powermove-3d-layers-${theme}.png`});
  }
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
    api.scene3d.edit({operation:'create',scene:{objects:[{id:'model',source:{assetId:asset.id},useSourceMaterials:true}],lights:[{id:'sun',type:'sun'}],camera:{p:{x:0,y:0,z:4}}}});
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
      {id:'floor',source:{primitive:'plane',parameters:{width:6,height:6}},p:{y:-.6,rx:-90},material:{p:{color:'#FFFFFF'}}}],
      lights:[{id:'key',type:'sun',p:{x:-3,y:5,z:4}}],environment:{p:{ambient:.15}}}});
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
