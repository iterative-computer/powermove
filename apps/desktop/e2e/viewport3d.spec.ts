import { expect, test } from './helpers/app';

/* The 3D view inside the ordinary composition viewer: 2D selection habits, a compact corner cluster, pinned cameras and the light ball. */

const API='viewport-proof';
const frames=(page:any)=>page.evaluate(()=>new Promise<void>(resolve=>requestAnimationFrame(()=>requestAnimationFrame(()=>resolve()))));
const center=async(locator:any)=>{const box=await locator.boundingBox();return {x:box!.x+box!.width/2,y:box!.y+box!.height/2};};
const drag=async(page:any,from:{x:number;y:number},dx:number,dy:number)=>{
  await page.mouse.move(from.x,from.y);await page.mouse.down();await page.mouse.move(from.x+dx,from.y+dy,{steps:8});await page.mouse.up();
};

test('2D compositions show no 3D controls, and adding 3D keeps the preview the same size',async({session})=>{
  await session.openEditor();const {page}=session;
  const solid=await page.evaluate(()=>{
    const PM=(window as any).PM;const result=PM.Edit.apply({type:'add_layer',layerType:'solid',name:'Plate',select:true});
    PM.invalidate();return result.data.results[0].data.layer.id;
  });
  await expect(page.locator('.vp-nav')).toBeHidden();
  await expect(page.locator('select[data-render-mode]')).toBeHidden();
  await frames(page);
  const before={stage:await page.locator('#stage').boundingBox(),frame:await page.locator('#stage-inner').boundingBox()};
  await page.evaluate(id=>{const PM=(window as any).PM;PM.Kernel.api('viewport-proof').scene3d.edit({operation:'add_object',object:{id:'cube',source:{primitive:'box'}}});PM.selectLayers([id]);PM.invalidate();},solid);
  await expect(page.locator('.vp-nav')).toBeVisible();
  await expect(page.locator('select[data-render-mode]')).toBeVisible();
  await frames(page);
  expect(await page.locator('#stage').boundingBox()).toEqual(before.stage);
  expect(await page.locator('#stage-inner').boundingBox()).toEqual(before.frame);
  // Models start at the composition centre, in the same pixels as 2D layers.
  expect(await page.evaluate(()=>{const p=(window as any).PM.L('cube').p;return [p['position.x'].v,p['position.y'].v,p['position.z'].v];})).toEqual([960,540,0]);
  // With a 2D layer selected, single keys keep their 2D meaning.
  const box=await page.locator('#stage').boundingBox(),tool=()=>page.evaluate(()=>(window as any).PM.Kernel.services.get('tool').tool);
  await page.mouse.move(box!.x+box!.width/2,box!.y+box!.height/2);await page.keyboard.press('g');await expect.poll(tool).toBe('pen');
  await page.keyboard.press('v');await expect.poll(tool).toBe('select');
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('3D layers select and drag like 2D layers, with selection-box handles to scale, turn and move along an axis',async({session})=>{
  await session.openEditor();const {page}=session;
  await page.evaluate(api=>{
    const PM=(window as any).PM,scene=PM.Kernel.api(api).scene3d;
    scene.edit({operation:'add_camera'});
    scene.edit({operation:'add_light',light:{id:'key',type:'point',p:{x:560,y:140,z:-400,intensity:30}}});
    scene.edit({operation:'add_object',object:{id:'cube',source:{primitive:'box'},material:{p:{color:'#ff7733'}}}});
    PM.selectLayers([]);PM.invalidate();
  },API);
  const value=(path:string)=>page.evaluate(path=>(window as any).PM.L('cube').p[path].v,path);
  const history=()=>page.evaluate(()=>(window as any).PM.hist.list().length);
  const frame=await page.locator('#stage-inner').boundingBox(),perPixel=1920/frame!.width,middle={x:frame!.x+frame!.width/2,y:frame!.y+frame!.height/2};

  // Click selects; the selection box appears in the 2D style.
  await page.mouse.click(middle.x,middle.y);
  expect(await page.evaluate(()=>(window as any).PM.sel.layers)).toEqual(['cube']);
  await expect(page.locator('.vp-gizmo [data-handle="corner"]')).toHaveCount(4);
  await expect(page.locator('.vp-gizmo [data-handle="rotate"]')).toHaveCount(1);
  for(const axis of ['x','y'])await expect(page.locator(`.vp-gizmo [data-handle="axis-${axis}"]`)).toHaveCount(1);

  // Dragging the model moves it under the pointer, one composition pixel per pixel at its depth, in one Undo.
  const steps=await history();
  await drag(page,middle,100,0);
  expect(await value('position.x')).toBeCloseTo(960+100*perPixel,-1);expect(await value('position.y')).toBeCloseTo(540,-1);expect(await value('position.z')).toBeCloseTo(0);
  expect(await history()).toBe(steps+1);
  await page.evaluate(()=>(window as any).PM.hist.undo());expect(await value('position.x')).toBe(960);

  // The y spoke moves along y only; Escape mid-drag puts it back.
  const spoke=await center(page.locator('.vp-gizmo [data-handle="axis-y"]'));
  await drag(page,spoke,40,60);
  expect(await value('position.y')).toBeGreaterThan(560);expect(await value('position.x')).toBe(960);
  await page.evaluate(()=>(window as any).PM.hist.undo());
  await page.mouse.move(spoke.x,spoke.y);await page.mouse.down();await page.mouse.move(spoke.x,spoke.y+50,{steps:5});
  await page.keyboard.press('Escape');await page.mouse.up();expect(await value('position.y')).toBe(540);

  // A corner scales and the stem turns, as on a 2D selection.
  const corner=await center(page.locator('.vp-gizmo [data-handle="corner"]').nth(2));
  await drag(page,corner,40,40);expect(await value('scale.x')).toBeGreaterThan(110);
  await page.evaluate(()=>(window as any).PM.hist.undo());
  const stem=await center(page.locator('.vp-gizmo [data-handle="rotate"]'));
  await drag(page,stem,120,120);expect(Math.abs(await value('rotation'))).toBeGreaterThan(20);

  for(const theme of ['dark','light']){
    await page.evaluate(theme=>{const PM=(window as any).PM;PM.theme.apply(theme);PM.invalidate();},theme);
    await frames(page);await page.locator('#gl').evaluate((canvas:HTMLCanvasElement)=>{canvas.toDataURL();});
    await page.screenshot({path:`/tmp/powermove-viewport3d-${theme}.png`});
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('cameras stay pinned to the top of the timeline, and the camera button switches between camera and free view',async({session})=>{
  await session.openEditor();const {page}=session;
  const order=await page.evaluate(api=>{
    const PM=(window as any).PM,scene=PM.Kernel.api(api).scene3d;
    scene.edit({operation:'add_object',object:{id:'cube',source:{primitive:'box'}}});
    scene.edit({operation:'add_camera',id:'shot'});
    scene.edit({operation:'add_object',object:{id:'ball',source:{primitive:'sphere'}}});
    PM.Edit.apply({type:'reorder_layer',target:'cube',index:0});PM.invalidate();
    return PM.proj.layers.map((l:any)=>l.id);
  },API);
  expect(order[0]).toBe('shot');expect(order[1]).toBe('cube');
  const view=()=>page.evaluate(api=>(window as any).PM.Kernel.api(api).scene3d.getView(),API);
  expect(await view()).toBe('camera');
  await page.locator('.vp-nav').getByRole('button',{name:'Camera view'}).click();
  await expect.poll(view).toBe('editor');
  await page.locator('.vp-nav').getByRole('button',{name:'Camera view'}).click();
  await expect.poll(view).toBe('camera');
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('the Lighting section applies presets in one Undo and aims a light by dragging its dot',async({session})=>{
  await session.openEditor();const {page}=session;
  await page.evaluate(api=>{
    const PM=(window as any).PM,scene=PM.Kernel.api(api).scene3d;
    scene.edit({operation:'add_object',object:{id:'cube',source:{primitive:'box'}}});PM.selectLayers(['cube']);PM.invalidate();
  },API);
  const section=page.locator('[data-inspector-section="3d-model"]');
  await expect(section.getByText('Lighting',{exact:true})).toBeVisible();
  const steps=await page.evaluate(()=>(window as any).PM.hist.list().length);
  expect(await page.evaluate(api=>(window as any).PM.Kernel.api(api).scene3d.viewport.run('lighting.preset','studio'),API)).toBe(true);
  expect(await page.evaluate(()=>(window as any).PM.hist.list().length)).toBe(steps+1);
  const dots=section.locator('.light-ball .light');await expect(dots).toHaveCount(3);
  const lights=()=>page.evaluate(api=>(window as any).PM.Kernel.api(api).scene3d.lighting.list(),API);
  const before=(await lights())[0];
  await section.locator('.light-ball').scrollIntoViewIfNeeded();
  const dot=await center(dots.first().locator('.mark'));
  await drag(page,dot,-25,30);
  const after=(await lights()).find((l:any)=>l.id===before.id);
  expect(Math.hypot(...after.direction.map((v:number,i:number)=>v-before.direction[i]))).toBeGreaterThan(.2);
  expect(await page.evaluate(()=>(window as any).PM.hist.list().length)).toBe(steps+2);
  await page.screenshot({path:'/tmp/powermove-viewport3d-lighting.png'});
  expect(session.diagnostics.pageErrors).toEqual([]);
});
