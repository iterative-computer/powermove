import { expect, test } from './helpers/app';
import { partsGlb } from './helpers/parts-glb';

/* Parts of one imported model, edited without splitting it into layers:
   the Properties tree, double-click picking in the viewer, the 3D Move
   handle moving only that part (one Undo), keyframes, and leaving part
   editing by selecting elsewhere. */

test('a model part is picked by double-click, moved by the gizmo, keyframed and undone', async ({ session }) => {
  test.setTimeout(120_000);
  await session.openEditor();
  const page = session.page;
  const model = await page.evaluate(async (bytes: number[]) => {
    const PM = (window as any).PM; PM.pause(); PM.AutomaticPreview?.cancel?.(); PM.setTime(0);
    PM.Kernel.api('parts-e2e').scene3d.edit({ operation: 'add_camera', camera: { p: { x: 0, y: 0, z: 3.2 } } });
    const imported = await PM.cmd('3d.import-obj', new File([new Uint8Array(bytes)], 'Console.glb', { type: 'model/gltf-binary' }));
    if (!imported.ok) throw new Error(imported.message);
    const id = PM.proj.layers.find((l: any) => l.d?.data?.object).id;
    PM.selectLayers([id]); PM.invalidate(); return id;
  }, [...partsGlb()]);
  const call = <T,>(fn: string, ...args: unknown[]) => page.evaluate(({ fn, args }) => (window as any).PM.Kernel.api('parts-e2e').scene3d[fn](...args) as T, { fn, args });

  // Properties lists the file's parts by their original names.
  const section = page.locator('[data-parts-section]');
  await expect(section).toBeVisible();
  // The lone wrapper opens by itself; nested parts wait behind their twirl.
  await expect(section.locator('.part-name')).toHaveText(['Whole Model', 'Console', 'Screen', 'JoyCon L', 'JoyCon R']);
  await section.getByRole('button', { name: 'Expand JoyCon R' }).click();
  await expect(section.locator('.part-name')).toHaveText(['Whole Model', 'Console', 'Screen', 'JoyCon L', 'JoyCon R', 'Stick R']);

  // Double-click on the right controller picks it (the single wrapper is skipped).
  const point = await page.evaluate((m: string) => {
    const PM = (window as any).PM, api = PM.Kernel.api('parts-e2e').scene3d, b = PM.GL.bounds(PM.L(m), PM.time);
    const inner = document.querySelector('#stage-inner')!.getBoundingClientRect();
    for (let x = b.x1 - 4; x > b.x0; x -= 4) {
      const y = (b.y0 + b.y1) / 2 + 40;
      if (api.pickPart(m, x, y) === '0_2') return { x: inner.left + x / PM.proj.w * inner.width, y: inner.top + y / PM.proj.h * inner.height };
    }
    throw new Error('JoyCon R not under any point');
  }, model);
  await page.mouse.dblclick(point.x, point.y);
  await expect.poll(() => call('getActivePart', model)).toBe('0_2');
  await expect(page.locator('[data-part-key="0_2"]')).toHaveAttribute('aria-selected', 'true');
  await expect(page.locator('[data-part-editor="0_2"]')).toBeVisible();

  // The Move handle drags only that part; the model layer itself stays put.
  const handle = await page.evaluate(() => {
    const viewer = (window as any).PM.Kernel.services.get('viewer'), rect = document.querySelector('#stage')!.getBoundingClientRect();
    for (let y = rect.top + 5; y < rect.bottom - 5; y += 3) for (let x = rect.left + 5; x < rect.right - 5; x += 3)
      if (viewer.sceneGizmo.hover(x, y) && (document.querySelector('[data-scene-gizmo]') as HTMLElement).dataset.gizmoAxis === 'X') return { x, y };
    throw new Error('No X handle on the part');
  });
  const history = await page.evaluate(() => (window as any).PM.hist.list().length);
  await page.mouse.move(handle.x, handle.y); await page.mouse.down();
  await page.mouse.move(handle.x + 120, handle.y, { steps: 10 }); await page.mouse.up();
  const moved = await call<any>('getPart', model, '0_2');
  expect(moved.installed).toBe(true);
  expect(moved.values.x).toBeGreaterThan(.05);
  expect(Math.abs(moved.values.y)).toBeLessThan(1e-6);
  expect(await page.evaluate((m: string) => (window as any).PM.L(m).p['position.x'].v, model)).toBe(0);
  expect(await page.evaluate(() => (window as any).PM.hist.list().length)).toBe(history + 1);
  await expect(page.locator('[data-part-key="0_2"] .part-dot')).toBeVisible();

  // One Undo puts the part back exactly where the file has it.
  await page.evaluate(() => (window as any).PM.hist.undo());
  expect((await call<any>('getPart', model, '0_2')).installed).toBe(false);

  // Keyframes: the diamond keys Position at 0s, a field edit at 1s keys again.
  await page.locator('[data-part-property="x,y,z"] .stopwatch').click();
  await page.evaluate(() => { (window as any).PM.setTime(1); (window as any).PM.invalidate(); });
  const field = page.locator('[data-part-field="x"] input');
  await field.click(); await field.fill('0.8'); await field.press('Enter');
  await expect.poll(async () => (await call<any>('getPart', model, '0_2', 1)).values.x).toBeCloseTo(.8, 5);
  expect((await call<any>('getPart', model, '0_2', .5)).values.x).toBeCloseTo(.4, 5);
  expect((await call<any>('getPart', model, '0_2', 0)).values.x).toBe(0);
  expect((await call<any>('getPart', model, '0_2')).channels.x.kf).toHaveLength(2);

  // Hiding a part from the tree; Reset clears everything on it.
  await page.locator('[data-part-key="0_1"]').hover();
  await page.locator('[data-part-key="0_1"] .part-eye').click();
  expect((await call<any>('getPart', model, '0_1')).values.visible).toBe(false);
  await page.locator('[data-part-editor] .part-reset').click();
  expect((await call<any>('getPart', model, '0_2')).installed).toBe(false);

  // Selecting something else leaves part editing.
  await page.evaluate(() => (window as any).PM.selectLayers([]));
  await expect.poll(() => call('getActivePart', model)).toBe(null);
});

test('groups selected model parts, animates the group, renames and reopens it',async({session},info)=>{
  test.setTimeout(120_000);await session.openEditor();const page=session.page;
  const id=await page.evaluate(async(bytes:number[])=>{
    const PM=(window as any).PM;PM.pause();PM.AutomaticPreview?.cancel?.();PM.replaceProject(PM.mkProject({w:640,h:360,dur:3,fps:30}));
    const api=PM.Kernel.api('part-groups-e2e').scene3d;api.edit({operation:'add_camera',camera:{p:{x:0,y:0,z:3.2}}});
    const result=await PM.cmd('3d.import-obj',new File([new Uint8Array(bytes)],'Console.glb',{type:'model/gltf-binary'}));if(!result.ok)throw new Error(result.message);
    const id=PM.proj.layers.find((layer:any)=>layer.d?.data?.object).id;PM.selectLayers([id]);PM.invalidate();return id;
  },[...partsGlb()]);
  const section=page.locator('[data-parts-section]'),groupButton=page.getByRole('button',{name:'Group selected parts',exact:true});
  await section.locator('[data-part-key="0_1"]').click();
  await section.locator('[data-part-key="0_2"]').click({modifiers:['Meta']});
  await expect(section.locator('[role="treeitem"][aria-selected="true"]')).toHaveCount(2);
  await expect(groupButton).toBeEnabled();await page.keyboard.press('Meta+g');
  const key=await page.evaluate(id=>(window as any).PM.Kernel.api('part-groups-e2e').scene3d.getActivePart(id),id);
  expect(key).toMatch(/^group_/);await expect(section.locator(`[data-part-editor="${key}"]`)).toBeVisible();
  const name=section.getByRole('textbox',{name:'Name',exact:true});await name.fill('Controllers');await name.press('Enter');
  await expect(section.locator(`[data-part-key="${key}"] .part-name`)).toHaveText('Controllers');
  const handle=await page.evaluate(()=>{
    const viewer=(window as any).PM.Kernel.services.get('viewer'),rect=document.querySelector('#stage')!.getBoundingClientRect();
    for(let y=rect.top+5;y<rect.bottom-5;y+=3)for(let x=rect.left+5;x<rect.right-5;x+=3)
      if(viewer.sceneGizmo.hover(x,y)&&(document.querySelector('[data-scene-gizmo]') as HTMLElement).dataset.gizmoAxis==='X')return{x,y};
    throw new Error('No X handle for the part group');
  });
  await page.mouse.move(handle.x,handle.y);await page.mouse.down();await page.mouse.move(handle.x+90,handle.y,{steps:8});await page.mouse.up();
  expect(await page.evaluate(({id,key})=>(window as any).PM.Kernel.api('part-groups-e2e').scene3d.getPart(id,key).values.x,{id,key})).toBeGreaterThan(.01);
  expect(await page.evaluate(id=>(window as any).PM.L(id).p['position.x'].v,id)).toBe(0);
  await page.evaluate(()=>(window as any).PM.hist.undo());
  await section.locator('[data-part-property="x,y,z"] .stopwatch').click();
  await page.evaluate(()=>{const PM=(window as any).PM;PM.setTime(1);PM.invalidate();});
  const x=section.locator('[data-part-field="x"] input');await x.click();await x.fill('.5');await x.press('Enter');
  const values=await page.evaluate(({id,key})=>{const api=(window as any).PM.Kernel.api('part-groups-e2e').scene3d;return [api.getPart(id,key,0).values.x,api.getPart(id,key,.5).values.x,api.getPart(id,key,1).values.x];},{id,key});
  expect(values).toEqual([0,.25,.5]);
  await expect(section.getByRole('button',{name:'Ungroup',exact:true})).toBeDisabled();
  for(const theme of ['light','dark']){
    await page.evaluate(theme=>(window as any).PM.theme.apply(theme),theme);
    await section.screenshot({path:info.outputPath(`part-groups-${theme}.png`)});
    expect(await page.locator('#panel-inspector').evaluate(el=>{
      const panel=el.getBoundingClientRect();return [...el.querySelectorAll('[data-parts-section] input')].every(field=>{const r=field.getBoundingClientRect();return r.left>=panel.left-1 && r.right<=panel.right+1;});
    })).toBe(true);
  }
  const filePath=session.userData+'/part-groups.pmv';
  await session.app.evaluate(({dialog},filePath)=>{dialog.showSaveDialog=async()=>({canceled:false,filePath});dialog.showOpenDialog=async()=>({canceled:false,filePaths:[filePath]});},filePath);
  await page.evaluate(async()=>{const PM=(window as any).PM;if(!await PM.saveProject())throw new Error('Save failed');await PM.openProject();});
  await page.waitForFunction(id=>{const PM=(window as any).PM;return !!PM.assets.get(PM.L(id)?.d?.data?.object?.source.assetId)?.object3d;},id);
  await page.evaluate(({id,key})=>{const PM=(window as any).PM;PM.selectLayers([id]);PM.Kernel.api('part-groups-e2e').scene3d.setActivePart(id,key);},{id,key});
  await expect(section.locator(`[data-part-key="${key}"] .part-name`)).toHaveText('Controllers');
  expect(await page.evaluate(({id,key})=>(window as any).PM.Kernel.api('part-groups-e2e').scene3d.getPart(id,key,1).values.x,{id,key})).toBe(.5);
  await section.locator('.part-reset').filter({hasText:'Reset'}).click();
  await expect(section.locator(`[data-part-key="${key}"]`)).toBeVisible();
  await x.click();await x.fill('.2');await x.press('Enter');
  await section.getByRole('button',{name:'Ungroup',exact:true}).click();
  await expect(section.locator(`[data-part-key="${key}"]`)).toHaveCount(0);
  await page.evaluate(()=>(window as any).PM.hist.undo());
  await expect(section.locator(`[data-part-key="${key}"]`)).toBeVisible();
  expect(session.diagnostics.pageErrors).toEqual([]);
});
