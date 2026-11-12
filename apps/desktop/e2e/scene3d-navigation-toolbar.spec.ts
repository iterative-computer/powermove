import { test, expect, chooseNativeMenu, inspectNativeMenu } from './helpers/app';

test('AE navigation toolbar variants, shortcuts, cancellation and render isolation',async({session})=>{
  await session.openEditor();const {page}=session;
  const before=await page.evaluate(()=>{
    const PM=(window as any).PM,api=PM.Kernel.api('camera-toolbar-test');
    api.scene3d.edit({operation:'add_camera',camera:{p:{x:0,y:0,z:6}}});
    api.scene3d.edit({operation:'add_object',object:{id:'box',source:{primitive:'box'},p:{x:.5}}});
    PM.selectLayers(['box']);PM.autokey=true;api.scene3d.setView('editor');PM.invalidate();
    return {layers:JSON.stringify(PM.proj.layers),history:PM.hist.list().length,pixels:[...PM.GL.renderToPixels(0,80,45,{transparent:true,mblur:false})]};
  });
  const toolbar=page.locator('#toolbar');
  await expect(toolbar.locator('[data-navigation]')).toHaveCount(3);
  const menu=await inspectNativeMenu(session,()=>toolbar.getByRole('button',{name:'Dolly tools',exact:true}).click());
  expect(menu.map(item=>item.label)).toEqual(['Dolly Towards Cursor','Dolly to Cursor','Dolly to Camera POI']);
  await chooseNativeMenu(session,'Orbit Around Scene',()=>toolbar.getByRole('button',{name:'Orbit tools',exact:true}).click());
  await expect(toolbar.locator('[data-navigation="orbit"]')).toHaveAttribute('aria-pressed','true');
  await expect(toolbar.locator('[data-tool="select"]')).toHaveAttribute('aria-pressed','false');
  const hash=()=>page.evaluate(()=>{
    const PM=(window as any).PM;PM.GL.render(0,{mblur:false,previewReuse:false});
    const gl=PM.GL.gl,p=new Uint8Array(gl.drawingBufferWidth*gl.drawingBufferHeight*4);gl.readPixels(0,0,gl.drawingBufferWidth,gl.drawingBufferHeight,gl.RGBA,gl.UNSIGNED_BYTE,p);
    let h=0;for(const v of p)h=(h*31+v)|0;return h;
  });
  const original=await hash(),box=(await page.locator('#stage').boundingBox())!,x=box.x+box.width*.65,y=box.y+box.height*.55;
  await page.mouse.move(x,y);await page.mouse.down();await page.mouse.move(x+65,y+25,{steps:8});await page.mouse.up();
  await expect.poll(hash).not.toBe(original);const orbited=await hash();
  await page.mouse.move(x,y);await page.mouse.down();await page.mouse.move(x+35,y+20,{steps:5});await page.keyboard.press('Escape');await page.mouse.up();expect(await hash()).toBe(orbited);
  await page.keyboard.press('Shift+1');await expect(toolbar.locator('[data-navigation="orbit"]')).toHaveAttribute('aria-label','Orbit Around Camera POI');
  await page.keyboard.press('2');await expect(toolbar.locator('[data-navigation="pan"]')).toHaveAttribute('aria-pressed','true');
  await page.keyboard.press('3');await expect(toolbar.locator('[data-navigation="dolly"]')).toHaveAttribute('aria-pressed','true');
  await page.keyboard.press('c');await expect(toolbar.locator('[data-navigation="orbit"]')).toHaveAttribute('aria-pressed','true');
  await page.keyboard.press('v');await expect(toolbar.locator('[data-tool="select"]')).toHaveAttribute('aria-pressed','true');
  await expect(toolbar.locator('[data-navigation="orbit"]')).toHaveAttribute('aria-pressed','false');
  const freeView=page.getByRole('tab',{name:'Free View',exact:true});
  await expect(freeView).toHaveAttribute('title',/⌘-click to reset/);
  await freeView.click();expect(await hash()).toBe(orbited);
  await freeView.click({modifiers:['Meta']});
  expect(await hash()).toBe(original);
  expect(await page.evaluate(()=>(window as any).PM.Kernel.api('camera-toolbar-test').scene3d.getView())).toBe('editor');
  await page.getByRole('tab',{name:'Active Camera',exact:true}).click();
  await freeView.click({modifiers:['Meta']});expect(await hash()).toBe(original);
  const after=await page.evaluate(()=>{
    const PM=(window as any).PM;return {layers:JSON.stringify(PM.proj.layers),history:PM.hist.list().length,pixels:[...PM.GL.renderToPixels(0,80,45,{transparent:true,mblur:false})]};
  });expect(after).toEqual(before);
  for(const theme of ['light','dark']){
    await page.evaluate(theme=>document.documentElement.dataset.theme=theme,theme);
    await toolbar.screenshot({path:`/tmp/powermove-navigation-toolbar-${theme}.png`});
    await freeView.locator('..').screenshot({path:`/tmp/powermove-free-view-${theme}.png`});
    for(const id of ['orbit','pan','dolly'])expect(await toolbar.locator(`[data-navigation="${id}"] svg`).count()).toBe(1);
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('toolbar edits the active shot camera with undo, keyframes and Free View isolation',async({session})=>{
  await session.openEditor();const {page}=session;
  await page.evaluate(()=>{
    const PM=(window as any).PM,api=PM.Kernel.api('shot-navigation-test');
    api.scene3d.edit({operation:'add_camera',id:'shot',camera:{p:{x:0,y:0,z:6}}});
    api.scene3d.edit({operation:'add_object',object:{id:'model',source:{primitive:'box',parameters:{width:1.8,depth:.5}},p:{x:.5,ry:25},material:{p:{color:'#F07030'}}}});
    PM.selectLayers(['model']);PM.autokey=false;PM.invalidate();
  });
  const state=()=>page.evaluate(()=>{
    const PM=(window as any).PM,api=PM.Kernel.api('shot-navigation-test');
    return {layers:JSON.stringify(PM.proj.layers),history:PM.hist.list().length,view:api.scene3d.getView(),selection:PM.sel.layers,
      pixels:[...PM.GL.renderToPixels(0,100,60,{transparent:true,mblur:false})]};
  });
  const before=await state(),stage=(await page.locator('#stage').boundingBox())!,x=stage.x+stage.width*.6,y=stage.y+stage.height*.5;
  const drag=async(dx:number,dy:number,cancel=false)=>{
    await page.mouse.move(x,y);await page.mouse.down();await page.mouse.move(x+dx,y+dy,{steps:8});
    if(cancel)await page.keyboard.press('Escape');await page.mouse.up();
  };
  const toolbar=page.locator('#toolbar');
  for(const [mode,dx,dy] of [['orbit',65,20],['pan',65,20],['dolly',0,-70]] as const){
    await toolbar.locator(`[data-navigation="${mode}"]`).click();
    expect((await state()).view).toBe('camera');
    await drag(dx,dy);const after=await state();
    expect(after.layers).not.toBe(before.layers);expect(after.pixels).not.toEqual(before.pixels);
    expect(after.history).toBe(before.history+1);expect(after.selection).toEqual(['model']);expect(after.view).toBe('camera');
    await page.evaluate(()=>(window as any).PM.hist.undo());expect((await state()).layers).toBe(before.layers);
    await page.evaluate(()=>(window as any).PM.hist.redo());expect((await state()).layers).toBe(after.layers);
    await page.evaluate(()=>(window as any).PM.hist.undo());
  }
  await toolbar.locator('[data-navigation="pan"]').click();const beforeCancel=await state();await drag(45,25,true);
  const cancelled=await state();expect(cancelled.layers).toBe(before.layers);expect(cancelled.history).toBe(beforeCancel.history);
  await page.evaluate(()=>{const PM=(window as any).PM;PM.setTime(1,{raw:true,force:true});PM.autokey=true;});
  await drag(45,0);
  const keyed=await page.evaluate(()=>{const PM=(window as any).PM;return {p:PM.L('shot').p['position.x'].kf,target:PM.L('shot').d.data.camera.p.targetX.kf};});
  expect(keyed.p).toHaveLength(1);expect(keyed.p[0].t).toBe(1);expect(keyed.target[0].t).toBe(1);
  await page.evaluate(()=>{const PM=(window as any).PM;PM.hist.undo();PM.setTime(0,{raw:true,force:true});PM.L('shot').lock=true;});
  const locked=await state();await drag(45,25);expect((await state()).layers).toBe(locked.layers);
  await page.getByRole('tab',{name:'Free View',exact:true}).click();await toolbar.locator('[data-navigation="orbit"]').click();await drag(55,20);
  const free=await state();expect(free.layers).toBe(locked.layers);expect(free.pixels).toEqual(locked.pixels);expect(free.view).toBe('editor');
  expect(session.diagnostics.pageErrors).toEqual([]);
});
