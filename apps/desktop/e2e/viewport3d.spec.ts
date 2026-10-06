import { expect, test } from './helpers/app';

/* Blender's 3D viewport over the composition, and the guarantee that 2D work keeps its own keys. */

test('2D compositions show no 3D viewport and keep their After Effects keys, even beside 3D layers',async({session})=>{
  await session.openEditor();const {page}=session;
  const solid=await page.evaluate(()=>{
    const PM=(window as any).PM;const result=PM.Edit.apply({type:'add_layer',layerType:'solid',name:'Plate',select:true});
    PM.invalidate();return result.data.results[0].data.layer.id;
  });
  await expect(page.locator('#composition-3d-controls')).toBeHidden();
  await expect(page.locator('.vp-nav')).toBeHidden();
  await expect(page.getByRole('radiogroup',{name:'3D tools'})).toHaveCount(0);
  const box=await page.locator('#stage').boundingBox(),center={x:box!.x+box!.width/2,y:box!.y+box!.height/2};
  const tool=()=>page.evaluate(()=>(window as any).PM.Kernel.services.get('tool').tool);
  await page.mouse.move(center.x,center.y);await page.keyboard.press('g');await expect.poll(tool).toBe('pen');
  await page.keyboard.press('v');await expect.poll(tool).toBe('select');
  // A 3D model in the same composition: with the 2D layer selected, keys stay After Effects keys.
  await page.evaluate(id=>{const PM=(window as any).PM;PM.Kernel.api('viewport-proof').scene3d.edit({operation:'add_object',object:{id:'cube',source:{primitive:'box'}}});PM.selectLayers([id]);PM.invalidate();},solid);
  await expect(page.locator('#composition-3d-controls')).toBeVisible();
  await page.mouse.move(center.x+3,center.y+3);await page.keyboard.press('g');await expect.poll(tool).toBe('pen');
  await page.keyboard.press('v');
  expect(await page.evaluate(()=>(window as any).PM.L('cube').p['position.x'].v)).toBe(0);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('Blender keys transform, duplicate, hide, select and key models over the viewport',async({session})=>{
  await session.openEditor();const {page}=session;
  const count=await page.evaluate(()=>{
    const PM=(window as any).PM,api=PM.Kernel.api('viewport-proof');
    api.scene3d.edit({operation:'add_camera',camera:{p:{x:0,y:1.5,z:7}}});
    api.scene3d.edit({operation:'add_light',light:{id:'key',type:'point',p:{x:-2,y:3,z:2,intensity:30}}});
    api.scene3d.edit({operation:'add_object',object:{id:'cube',source:{primitive:'box'},material:{p:{color:'#ff7733'}}}});
    PM.selectLayers(['cube']);PM.invalidate();return PM.hist.list().length;
  });
  const box=await page.locator('#stage').boundingBox(),center={x:box!.x+box!.width/2,y:box!.y+box!.height/2};
  const value=(path:string,id='cube')=>page.evaluate(({path,id})=>(window as any).PM.L(id).p[path].v,{path,id});
  const state=()=>page.evaluate(()=>(window as any).PM.Kernel.api('viewport-proof').scene3d.viewport.state());
  await page.mouse.move(center.x,center.y);
  await expect.poll(async()=>(await state()).inContext).toBe(true);

  // G X 2 Enter, R Z 90 Enter, S 2 Enter — each one Undo.
  await page.keyboard.press('g');await expect(page.locator('.vp-op-header')).toBeVisible();
  await page.keyboard.press('x');await page.keyboard.press('2');
  await expect(page.locator('.vp-op-header')).toHaveText('D: [2|] (2) along global X');
  await expect(page.locator('.vp-op-hints')).toContainText('Confirm');
  await page.keyboard.press('Enter');
  expect(await value('position.x')).toBeCloseTo(2);
  await page.keyboard.press('r');await page.keyboard.press('z');for(const k of '90')await page.keyboard.press(k);await page.keyboard.press('Enter');
  expect(await value('rotation')).toBeCloseTo(90);
  await page.keyboard.press('s');await page.keyboard.press('2');await page.keyboard.press('Enter');
  expect(await value('scale.x')).toBeCloseTo(200);
  expect(await page.evaluate(()=>(window as any).PM.hist.list().length)).toBe(count+3);

  // Pointer-driven grab: right click cancels, left click confirms.
  await page.keyboard.press('g');await page.mouse.move(center.x+120,center.y,{steps:6});
  await expect.poll(()=>value('position.x')).toBeGreaterThan(2.2);
  await page.mouse.down({button:'right'});await page.mouse.up({button:'right'});
  expect(await value('position.x')).toBeCloseTo(2);
  await page.mouse.move(center.x,center.y);await page.keyboard.press('g');await page.keyboard.press('Shift+z');
  await page.mouse.move(center.x+60,center.y-40,{steps:6});await page.mouse.down();await page.mouse.up();
  expect(await value('position.z')).toBeCloseTo(0);

  // Alt+G/R/S clear, I keys, Shift+D duplicates (Escape keeps the copy in place).
  await page.keyboard.press('Alt+g');await page.keyboard.press('Alt+r');await page.keyboard.press('Alt+s');
  expect([await value('position.x'),await value('rotation'),await value('scale.x')]).toEqual([0,0,100]);
  await page.keyboard.press('i');expect(await page.evaluate(()=>(window as any).PM.L('cube').p['position.x'].kf.length)).toBe(1);
  await page.keyboard.press('Shift+d');await page.keyboard.press('Escape');
  const copies=await page.evaluate(()=>(window as any).PM.proj.layers.filter((l:any)=>l.d?.definition==='powermove.3d.object').length);
  expect(copies).toBe(2);

  // A selects all 3D layers, Alt+A none; H hides and Alt+H reveals.
  await page.keyboard.press('a');expect((await state()).selected.length).toBe(4);
  await page.keyboard.press('Alt+a');expect((await state()).selected).toEqual([]);
  await page.evaluate(()=>(window as any).PM.selectLayers(['cube']));
  await page.keyboard.press('h');expect(await page.evaluate(()=>(window as any).PM.L('cube').on)).toBe(false);
  await page.keyboard.press('Alt+h');expect(await page.evaluate(()=>(window as any).PM.L('cube').on)).toBe(true);

  // Shift+right click places the 3D cursor; the light's wire selects with a click.
  await page.mouse.move(center.x-80,center.y+40);await page.keyboard.down('Shift');await page.mouse.down({button:'right'});await page.mouse.up({button:'right'});await page.keyboard.up('Shift');
  expect((await state()).cursor.some((v:number)=>Math.abs(v)>1e-3)).toBe(true);

  // Z opens the shading pie; choosing Wireframe changes the viewport only.
  await page.mouse.move(center.x,center.y);await page.keyboard.press('z');
  const pie=page.getByRole('menu',{name:'Shading'});await expect(pie).toBeVisible();
  await pie.getByRole('menuitemradio',{name:'Wireframe'}).click();await expect(pie).toBeHidden();
  expect((await state()).shading).toBe('wireframe');
  await expect(page.getByRole('radio',{name:'Wireframe',exact:true})).toHaveAttribute('aria-checked','true');
  await page.keyboard.press('Shift+z');expect((await state()).shading).toBe('solid');

  // Orbit into a user view to show the floor grid, outlines and light/camera wires.
  await page.keyboard.press('a');await page.evaluate(()=>(window as any).PM.selectLayers(['cube']));
  await page.mouse.move(center.x,center.y);await page.mouse.down({button:'middle'});await page.mouse.move(center.x-90,center.y-40,{steps:8});await page.mouse.up({button:'middle'});
  await page.keyboard.press('Home');
  await expect(page.locator('.vp-label span').first()).toHaveText('User Perspective');
  for(const theme of ['dark','light']){
    await page.evaluate(theme=>{const PM=(window as any).PM;PM.theme.apply(theme);PM.invalidate();},theme);
    await page.evaluate(()=>new Promise<void>(resolve=>requestAnimationFrame(()=>requestAnimationFrame(()=>resolve()))));
    await page.locator('#gl').evaluate((canvas:HTMLCanvasElement)=>{canvas.toDataURL();});
    await page.screenshot({path:`/tmp/powermove-viewport3d-${theme}.png`});
  }
  await page.mouse.move(center.x,center.y);await page.keyboard.press('`');
  await expect(page.getByRole('menu',{name:'View'})).toBeVisible();
  await page.screenshot({path:'/tmp/powermove-viewport3d-pie.png'});
  await page.keyboard.press('Escape');
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('Blender Properties tabs, B box select, X-ray, Walk, Local View, Zoom Border and area lights',async({session})=>{
  await session.openEditor();const {page}=session;
  await page.evaluate(()=>{
    const PM=(window as any).PM,api=PM.Kernel.api('viewport-extras');
    api.scene3d.edit({operation:'add_camera',camera:{p:{x:0,y:1.5,z:7}}});
    api.scene3d.edit({operation:'add_light',light:{id:'panel',type:'area',p:{x:0,y:4,z:2,width:3,height:1,intensity:40}}});
    for(const [id,x] of [['left',-1.5],['right',1.5]] as const)api.scene3d.edit({operation:'add_object',object:{id,source:{primitive:'box'},p:{x},material:{p:{color:'#ff7733'}}}});
    PM.selectLayers(['left']);PM.invalidate();
  });
  const box=await page.locator('#stage').boundingBox(),center={x:box!.x+box!.width/2,y:box!.y+box!.height/2};
  const state=()=>page.evaluate(()=>(window as any).PM.Kernel.api('viewport-extras').scene3d.viewport.state());

  // Properties: Blender's context tabs; a model opens on Material and remembers the chosen tab.
  const tabs=page.getByRole('tablist',{name:'3D properties'});await expect(tabs).toBeVisible();
  for(const name of ['Render Properties','World Properties','Object Properties','Material Properties'])await expect(tabs.getByRole('tab',{name})).toBeVisible();
  await tabs.getByRole('tab',{name:'World Properties'}).click();
  await expect(page.getByRole('tabpanel',{name:'World Properties'})).toBeVisible();
  await page.evaluate(()=>{const PM=(window as any).PM;PM.selectLayers([PM.proj.layers.find((l:any)=>l.d?.definition==='powermove.3d.light').id]);});
  await expect(page.getByRole('tab',{name:'Data Properties'})).toBeVisible();
  await expect(page.getByRole('tabpanel',{name:'World Properties'})).toBeVisible();
  await page.getByRole('tab',{name:'Data Properties'}).click();
  await expect(page.getByRole('tabpanel',{name:'Data Properties'}).getByText('Size X')).toBeVisible();

  // B arms a box select that starts on top of a model.
  await page.evaluate(()=>(window as any).PM.selectLayers([]));
  await page.mouse.move(center.x,center.y);await page.keyboard.press('b');
  await expect(page.locator('.vp-op-header')).toHaveText('Box Select');
  await page.mouse.move(box!.x+box!.width*.15,box!.y+box!.height*.2);await page.mouse.down();
  await page.mouse.move(box!.x+box!.width*.85,box!.y+box!.height*.8,{steps:6});await page.mouse.up();
  await expect.poll(async()=>(await state()).selected.length).toBeGreaterThanOrEqual(2);
  await expect(page.locator('.vp-op-header')).toBeHidden();

  // Alt+Z X-ray changes the editor image but never the output.
  const output=()=>page.evaluate(()=>[...(window as any).PM.GL.renderToPixels(0,96,54,{transparent:true,mblur:false,draft3d:true})].join(','));
  const editorImage=()=>page.evaluate(()=>[...(window as any).PM.GL.renderToPixels(0,96,54,{transparent:true,mblur:false,editorViewport:true})].join(','));
  const [out,edit]=[await output(),await editorImage()];
  await page.mouse.move(center.x,center.y);await page.keyboard.press('Alt+z');expect((await state()).xray).toBe(true);
  await expect(page.getByRole('button',{name:'X-ray'})).toHaveAttribute('aria-pressed','true');
  expect(await editorImage()).not.toBe(edit);expect(await output()).toBe(out);
  await page.keyboard.press('Alt+z');

  // Numpad / isolates the selection in Local View; again restores.
  await page.evaluate(()=>(window as any).PM.selectLayers(['right']));
  await page.mouse.move(center.x,center.y);await page.keyboard.press('/');
  await expect(page.locator('.vp-label span').first()).toHaveText('User Perspective (Local)');
  expect(await page.evaluate(()=>{const PM=(window as any).PM,b=PM.GL.bounds(PM.L('left'),0);return b?PM.GL.pick((b.x0+b.x1)/2,(b.y0+b.y1)/2,0)?.id??null:null;})).not.toBe('left');
  await page.keyboard.press('/');await expect(page.locator('.vp-label span').first()).not.toContainText('Local');

  // Shift+B zooms to a dragged region.
  await page.keyboard.press('Shift+b');await expect(page.locator('.vp-op-header')).toHaveText('Zoom Border');
  const before=await page.evaluate(()=>(window as any).PM.Kernel.services.get('viewer').shown);
  await page.mouse.move(center.x-60,center.y-40);await page.mouse.down();await page.mouse.move(center.x+60,center.y+40,{steps:5});await page.mouse.up();
  await expect(page.locator('.vp-op-header')).toBeHidden();
  expect(await page.evaluate(()=>(window as any).PM.Kernel.services.get('viewer').shown)).toBeGreaterThan(before);
  await page.evaluate(()=>(window as any).PM.Kernel.services.get('viewer').returnToComposition());

  // Shift+` walks a user view forward; Escape returns to where it started.
  await page.keyboard.press('7');const start=await page.evaluate(()=>(window as any).PM.Kernel.api('viewport-extras').scene3d.viewport.state().view.label);
  expect(start).toBe('Top Orthographic');
  await page.mouse.move(center.x,center.y);await page.keyboard.press('Shift+Backquote');
  await expect(page.locator('.vp-op-header')).toContainText('Walk');
  await page.keyboard.down('w');await page.waitForTimeout(250);await page.keyboard.up('w');
  await page.keyboard.press('Escape');await expect(page.locator('.vp-op-header')).toBeHidden();
  await page.keyboard.press('Shift+Backquote');await page.keyboard.down('w');await page.waitForTimeout(250);await page.keyboard.up('w');
  await page.mouse.down();await page.mouse.up();
  await expect(page.locator('.vp-label span').first()).toHaveText('User Perspective');
  await page.screenshot({path:'/tmp/powermove-viewport3d-extras.png'});
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('Edit Mode: Tab, box-select points, G and E edit the real mesh with Undo',async({session})=>{
  await session.openEditor();const {page}=session;
  const count=await page.evaluate(()=>{
    const PM=(window as any).PM,api=PM.Kernel.api('edit-mode');
    api.scene3d.edit({operation:'add_camera',camera:{p:{x:2.5,y:2.5,z:5}}});
    api.scene3d.edit({operation:'add_object',object:{id:'cube',source:{primitive:'box'},material:{p:{color:'#ff7733'}}}});
    PM.selectLayers(['cube']);PM.invalidate();return PM.hist.list().length;
  });
  const box=await page.locator('#stage').boundingBox(),center={x:box!.x+box!.width/2,y:box!.y+box!.height/2};
  const state=()=>page.evaluate(()=>(window as any).PM.Kernel.api('edit-mode').scene3d.viewport.state());
  await page.mouse.move(center.x,center.y);await page.keyboard.press('Tab');
  await expect(page.getByRole('button',{name:'Interaction mode: Edit Mode'})).toBeVisible();
  expect((await state()).editMode).toMatchObject({total:8,selected:0});
  // Box-select the upper half of the cube's points (the camera looks slightly down on it).
  await page.mouse.move(box!.x+5,box!.y+5);await page.mouse.down();await page.mouse.move(box!.x+box!.width-5,center.y-2,{steps:6});await page.mouse.up();
  await expect.poll(async()=>(await state()).editMode?.selected).toBeGreaterThan(0);
  await page.mouse.move(center.x,center.y);
  await page.keyboard.press('g');await page.keyboard.press('y');await page.keyboard.press('1');await page.keyboard.press('Enter');
  const mesh=()=>page.evaluate(()=>(window as any).PM.L('cube').d.data.object.source.mesh);
  expect(await mesh()).toBeTruthy();
  await page.keyboard.press('e');await expect(page.locator('.vp-op-header')).toContainText('along local Z');
  await page.keyboard.press('1');await page.keyboard.press('Enter');
  expect((await mesh()).indices.length).toBeGreaterThan(36);
  expect(await page.evaluate(()=>(window as any).PM.hist.list().length)).toBe(count+2);
  await page.evaluate(()=>{const PM=(window as any).PM;PM.theme.apply('dark');PM.invalidate();});
  await page.evaluate(()=>new Promise<void>(r=>requestAnimationFrame(()=>requestAnimationFrame(()=>r()))));
  await page.locator('#gl').evaluate((c:HTMLCanvasElement)=>{c.toDataURL();});
  await page.screenshot({path:'/tmp/powermove-viewport3d-edit.png'});
  await page.keyboard.press('Tab');expect((await state()).editMode).toBeNull();
  await page.evaluate(()=>{const PM=(window as any).PM;PM.hist.undo();PM.hist.undo();});
  expect(await page.evaluate(()=>(window as any).PM.L('cube').d.data.object.source.primitive)).toBe('box');
  expect(session.diagnostics.pageErrors).toEqual([]);
});
