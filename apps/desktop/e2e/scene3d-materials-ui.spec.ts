import { mkdir } from 'node:fs/promises';
import { expect, test } from './helpers/app';
import { twoMaterialGlb } from './helpers/two-material-glb';

/* Layers as Materials, through the interface: the Materials section in
   Properties, the source picker (search, Comps / Footage filter, loop-disabled
   sources), Material Options keyframing, the Timeline's Materials group, the
   "used as material" badge and dropping a Project item on a slot.
   Set PM_SHOT_DIR to also write the review screenshots. */

const shots = process.env.PM_SHOT_DIR;
async function shot(page: any, name: string, clip?: { x: number; y: number; width: number; height: number }) {
  if (!shots) return;
  await mkdir(shots, { recursive: true });
  await page.waitForTimeout(150);
  await page.screenshot({ path: `${shots}/${name}.png`, clip });
}

async function setup(session: any) {
  await session.openEditor();
  const page = session.page;
  const ids = await page.evaluate(async (bytes: number[]) => {
    const PM = (window as any).PM; PM.pause(); PM.AutomaticPreview?.cancel?.();
    const api = PM.Kernel.api('materials-ui-e2e');
    api.scene3d.edit({ operation: 'add_camera', camera: { p: { x: 0, y: 0, z: 2.6 } } });
    const main = PM.proj.compId;
    // An animated pre-comp: red at 0s, blue from 1s.
    const comp = PM.Comps.create({ name: 'Screen Animation', w: 256, h: 256, fps: 30, dur: 3 }, { open: true });
    PM.Edit.apply({ type: 'add_layer', id: 'inside', layerType: 'solid', content: { w: 256, h: 256, color: '#2F80ED' }, properties: { 'position.x': 128, 'position.y': 128, 'anchor.x': 128, 'anchor.y': 128 } });
    PM.Edit.apply({ type: 'replace_keyframes', target: 'inside', path: 'c.color', keyframes: [{ time: 0, value: '#2F80ED', hold: true }, { time: 1, value: '#27AE60', hold: true }] });
    PM.Edit.apply(['position.x', 'position.y'].map(path => ({ type: 'set_property', target: 'inside', path, value: 128 })));
    PM.Comps.open(main);
    PM.Edit.apply({ type: 'add_layer', id: 'anim', layerType: 'precomp', name: 'Screen Animation', visible: false, content: { comp, w: 256, h: 256 } });
    PM.Edit.apply({ type: 'add_layer', id: 'paint', layerType: 'solid', name: 'Red Solid', visible: false, content: { w: 256, h: 256, color: '#E5484D' } });
    PM.Edit.apply({ type: 'add_layer', id: 'title', layerType: 'text', name: 'Title', visible: false, content: { text: 'HELLO', size: 64, color: '#FFFFFF' } });
    PM.Edit.apply({type:'add_layer',id:'artwork',layerType:'group',name:'Screen Artwork',visible:false});
    PM.Edit.apply({type:'add_layer',id:'group-paint',layerType:'shape',content:{shape:'rect',w:128,h:128,radius:0,color:'#2F80ED'}});
    PM.Edit.apply({type:'move_to_group',targets:['group-paint'],group:'artwork'});
    const imported = await PM.cmd('3d.import-obj', new File([new Uint8Array(bytes)], 'phone.glb', { type: 'model/gltf-binary' }));
    if (!imported.ok) throw new Error(imported.message);
    const model = PM.proj.layers.find((l: any) => l.d?.data?.object).id;
    // A solid blurred by the model (Compound Blur's Blur Layer): using it on the model would loop.
    PM.Edit.apply({ type: 'add_layer', id: 'matted', layerType: 'solid', name: 'Blurred by Phone', visible: false, content: { w: 64, h: 64, color: '#FFFFFF' } });
    const fx = PM.mkEffect('compoundBlur', undefined, PM.L('matted')); fx.inputs = { map: { layerId: model, stage: 'source' } };
    PM.L('matted').fx = [fx]; PM.touch();
    const canvas = document.createElement('canvas'); canvas.width = canvas.height = 8;
    const ctx = canvas.getContext('2d')!; ctx.fillStyle = '#F2C94C'; ctx.fillRect(0, 0, 8, 8);
    const blob = await new Promise<Blob>(resolve => canvas.toBlob(b => resolve(b!)));
    const image = await PM.assets.add(new File([blob], 'Yellow Swatch.png', { type: 'image/png' }), { silent: true });
    PM.selectLayers([model]);
    return { model, comp, image: image.id };
  }, [...twoMaterialGlb()]);
  await page.waitForTimeout(500);
  return ids;
}

const maps = (page: any, id: string) => page.evaluate((id: string) => {
  const object = (window as any).PM.L(id).d.data.object; return { whole: object.material.maps, own: object.sourceMaterials };
}, id);

test('Materials section picks layers, comps and images per material, and keyframes Material Options', async ({ session }) => {
  test.setTimeout(120_000);
  const page = session.page;
  const { model } = await setup(session);
  const section = page.locator('[data-materials-section]');
  await expect(section).toBeVisible();
  await expect(section.locator('[data-material-index]')).toHaveCount(3);
  await expect(section.locator('.mat-name')).toHaveText(['Whole Model', 'Screen', 'Body']);
  await section.evaluate((el: Element) => el.previousElementSibling?.scrollIntoView({ block: 'start' }));
  await shot(page, '02-after-01-inspector-materials-light');

  // Open Screen; Whole Model stays open as well.
  await section.locator('[data-material-index="0"] .mat-twirl').click();
  const screen = section.locator('[data-material-index="0"]');
  const trigger = screen.locator('[data-material-slot="0:color"] button');
  await trigger.scrollIntoViewIfNeeded();
  await trigger.click();
  const picker = page.locator('[data-material-picker="color"]');
  await expect(picker).toBeVisible();
  await expect(picker.locator('.pm-menu-heading')).toHaveText(['Pre-comp Layers', 'Groups', 'Solid Layers', 'Text & Shape Layers', 'Other Layers', 'Compositions', 'Images']);
  // Hidden layers still work, and say so.
  await expect(picker.locator('[data-option="layer:paint"] .mp-tag')).toHaveAttribute('title', 'Hidden layers still work as materials');
  // Loops are disabled with the reason.
  const matted = picker.locator('[data-option="layer:matted"]');
  await expect(matted).toHaveAttribute('aria-disabled', 'true');
  await expect(matted.locator('.pm-menu-detail')).toHaveText('Uses this model, so it would loop');
  await expect(picker.locator('[data-option^="layer:"][aria-disabled="true"]').filter({ hasText: 'Camera' }).locator('.pm-menu-detail')).toHaveText('3D layers can’t be painted onto a model');
  await shot(page, '02-after-02-picker-open-groups');
  await matted.scrollIntoViewIfNeeded();
  await shot(page, '02-after-03-picker-loop-disabled');

  // A hidden group assigns directly, without making a pre-comp, and undoes once.
  await picker.locator('[data-option="layer:artwork"]').click();
  await expect.poll(async()=>(await maps(page,model)).own?.[0]?.color).toEqual({layerId:'artwork'});
  await expect(trigger).toContainText('Screen Artwork');
  await page.keyboard.press('Meta+z');
  await expect.poll(async()=>(await maps(page,model)).own?.[0]?.color).toBeUndefined();
  await trigger.click();await expect(picker).toBeVisible();

  // Comps filter: pre-comp layers and compositions; the open comp would loop.
  await picker.getByRole('radio', { name: 'Comps' }).click();
  await expect(picker.locator('.pm-menu-heading')).toHaveText(['Pre-comp Layers', 'Compositions']);
  await expect(picker.locator('[data-option^="comp:"][aria-disabled="true"] .pm-menu-detail')).toHaveText('This model is in this comp, so it would loop');
  await shot(page, '02-after-04-picker-comps-filter');
  await picker.getByRole('radio', { name: 'All' }).click();

  // Search narrows; Enter picks the first match.
  await page.keyboard.type('red so');
  await expect(picker.locator('[data-option^="layer:"]')).toHaveCount(1);
  await shot(page, '02-after-05-picker-search');
  await page.keyboard.press('Enter');
  await expect(picker).toHaveCount(0);
  await expect.poll(async () => (await maps(page, model)).own?.[0]?.color).toEqual({ layerId: 'paint' });
  await expect(trigger).toContainText('Red Solid');
  await page.evaluate(() => (window as any).PM.invalidate?.());
  await shot(page, '02-after-06-solid-on-screen-material');

  // A loop picked anyway (agent or race) is refused gently and nothing changes.
  const refused = await page.evaluate((id: string) => (window as any).PM.Kernel.api('materials-ui-e2e').scene3d.setMaterialMap(id, 0, 'color', { layerId: 'matted' }), model);
  expect(refused.ok).toBe(false);

  // Body takes the animated pre-comp; Whole Model keeps Original.
  await section.locator('[data-material-index="1"] .mat-twirl').click();
  const bodyTrigger = section.locator('[data-material-slot="1:color"] button');
  await bodyTrigger.scrollIntoViewIfNeeded();
  await bodyTrigger.click();
  await picker.locator('[data-option="layer:anim"]').click();
  await expect.poll(async () => (await maps(page, model)).own?.[1]?.color).toEqual({ layerId: 'anim' });
  await expect(bodyTrigger).toContainText('Screen Animation');
  await page.evaluate(() => { const PM = (window as any).PM; PM.setTime(0); PM.invalidate?.(); });
  await shot(page, '02-after-07-precomp-on-body-0s');
  await page.evaluate(() => { const PM = (window as any).PM; PM.setTime(1.2); PM.invalidate?.(); });
  await shot(page, '02-after-08-precomp-on-body-1s');
  const pixels = await page.evaluate(async () => {
    const PM = (window as any).PM; const read = async (t: number) => { await PM.prepareFrame(t, { mblur: false }); const c = PM.renderFrameTo(t, 480, 270, { mblur: false }); return [...c.getContext('2d').getImageData(300, 135, 1, 1).data]; };
    return { early: await read(0), late: await read(1.2) };
  });
  expect(pixels.early[2]).toBeGreaterThan(pixels.early[1]);
  expect(pixels.late[1]).toBeGreaterThan(pixels.late[2]);

  // Original restores.
  await bodyTrigger.click();
  await picker.locator('[data-option="original"]').click();
  await expect.poll(async () => (await maps(page, model)).own?.[1]?.color).toBeUndefined();

  // Material Options: the stopwatch installs and keys an inheriting property.
  await page.evaluate(() => { const PM = (window as any).PM; PM.setTime(0); });
  const opacity = screen.locator('[data-material-property="mi.0.opacity"]');
  await opacity.scrollIntoViewIfNeeded();
  await expect(opacity).toHaveAttribute('title', /see-through/);
  await opacity.getByRole('button', { name: 'Add keyframe for Opacity' }).click();
  await page.evaluate(() => { const PM = (window as any).PM; PM.setTime(1); });
  const field = opacity.locator('input').first();
  await field.click(); await field.fill('40'); await field.press('Enter');
  const channel = await page.evaluate((id: string) => (window as any).PM.L(id).d.data.object.sourceMaterialProperties?.[0]?.opacity, model);
  expect(channel.kf.map((k: any) => [k.t, k.v])).toEqual([[0, 100], [1, 40]]);
  await shot(page, '02-after-09-material-options-keyframed');

  // Dropping a Project image on a slot assigns it.
  const dropped = await page.evaluate(({ id }: { id: string }) => {
    const PM = (window as any).PM, image = Object.entries<any>(PM.proj.assets).find(([, a]) => a.kind === 'image')!;
    const slot = document.querySelector('[data-material-index="all"] [data-material-slot="all:emissive"]')!;
    const data = new DataTransfer(); data.setData('application/x-powermove-asset', JSON.stringify({ id: image[0], name: image[1].name, kind: 'image' }));
    slot.dispatchEvent(new DragEvent('dragover', { dataTransfer: data, bubbles: true, cancelable: true }));
    slot.dispatchEvent(new DragEvent('drop', { dataTransfer: data, bubbles: true, cancelable: true }));
    return PM.L(id).d.data.object.material.maps.emissive === image[0];
  }, { id: model });
  expect(dropped).toBe(true);

  // Reset puts a material back exactly, in one undo step.
  await screen.locator('.mat-reset').click();
  expect((await maps(page, model)).own?.[0]).toBeUndefined();
  await page.evaluate(() => (window as any).PM.hist.undo());
  expect((await maps(page, model)).own?.[0]?.color).toEqual({ layerId: 'paint' });

  await page.evaluate(() => (window as any).PM.theme.apply('dark'));
  await section.locator('[data-material-index="0"]').scrollIntoViewIfNeeded();
  await shot(page, '02-after-10-inspector-materials-dark');
  await trigger.click();
  await shot(page, '02-after-11-picker-dark');
  // The trigger toggles; Escape closes too.
  await trigger.click();
  await expect(picker).toHaveCount(0);
  await trigger.click();
  await expect(picker).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(picker).toHaveCount(0);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('Timeline Materials group, Material Options rows and the used-as-material badge', async ({ session }) => {
  test.setTimeout(120_000);
  const page = session.page;
  const { model } = await setup(session);
  await page.evaluate((id: string) => {
    const api = (window as any).PM.Kernel.api('materials-ui-e2e');
    api.scene3d.setMaterialMap(id, 0, 'color', { layerId: 'paint' });
    api.scene3d.setMaterialProperties(id, 1, { roughness: { v: .2, kf: [{ t: 0, v: .2 }, { t: 2, v: .9 }], expr: null } });
  }, model);
  const cvBox = () => page.evaluate(() => { const r = (window as any).PM.Kernel.services.get('timeline').cv.getBoundingClientRect(); return { x: r.left, y: r.top }; });
  const rows = () => page.evaluate(() => {
    const T = (window as any).PM.Kernel.services.get('timeline');
    return { rows: T.rows.map((r: any) => ({ kind: r.kind, key: r.key, label: r.label, layer: r.L?.id, virtual: !!r.virtual })), ruler: T.ruler, row: T.row, scrollY: T.scrollY, gut: T.gut, valueX: T.propertyValueX };
  });
  const point = async (match: (r: any) => boolean, x: (s: any) => number) => {
    const state = await rows(); const index = state.rows.findIndex(match);
    expect(index).toBeGreaterThanOrEqual(0);
    const box = await cvBox();
    return { x: box.x + x(state), y: box.y + state.ruler + index * state.row - state.scrollY + state.row / 2 };
  };

  // The model always has a twirl; opening it shows the Materials group.
  const collapsed = await page.evaluate((id: string) => (window as any).PM.Kernel.api('materials-ui-e2e').uiState.getLayerCollapsed((window as any).PM.L(id)), model);
  if (collapsed) {
    const twirl = await point(r => r.kind === 'layer' && r.layer === model, () => 87);
    await page.mouse.click(twirl.x, twirl.y);
  }
  await expect.poll(async () => (await rows()).rows.filter((r: any) => r.kind === 'material').map((r: any) => r.label)).toEqual(['Whole Model', 'Screen', 'Body']);
  // Body's animated Roughness shows even while Body is closed.
  const bodyRoughness = 'mi.1.roughness';
  expect((await rows()).rows.some((r: any) => r.key === bodyRoughness)).toBe(true);
  await shot(page, '02-after-12-timeline-materials-group');

  // Open Screen: Emissive and all six options (inheriting ones as stand-ins).
  const screenRow = await point(r => r.kind === 'material' && r.key === 'material:0:color', () => 148);
  await page.mouse.click(screenRow.x, screenRow.y);
  await expect.poll(async () => (await rows()).rows.filter((r: any) => r.key?.startsWith('mi.0.')).length).toBe(6);
  expect((await rows()).rows.find((r: any) => r.key === 'material:0:emissive')?.label).toBe('Emissive');
  await shot(page, '02-after-13-timeline-material-options');

  // A stand-in row's diamond installs and keys the property.
  const metallic = await point(r => r.key === 'mi.0.metallic', () => 148.5);
  await page.mouse.click(metallic.x, metallic.y);
  await expect.poll(() => page.evaluate((id: string) => (window as any).PM.L(id).d.data.object.sourceMaterialProperties?.[0]?.metallic?.kf?.length ?? 0, model)).toBe(1);
  expect((await rows()).rows.find((r: any) => r.key === 'mi.0.metallic')?.virtual).toBe(false);

  // The Base Color pop-up opens the same picker.
  const popup = await point(r => r.key === 'material:0:color', s => s.valueX + 30);
  await page.mouse.click(popup.x, popup.y);
  const picker = page.locator('[data-material-picker="color"]');
  await expect(picker).toBeVisible();
  await expect(picker.locator('[data-option="layer:paint"]')).toHaveAttribute('aria-selected', 'true');
  await shot(page, '02-after-14-timeline-picker');
  await picker.locator('[data-option="layer:title"]').click();
  await expect.poll(() => page.evaluate((id: string) => (window as any).PM.L(id).d.data.object.sourceMaterials?.[0]?.color?.layerId, model)).toBe('title');

  // The source wears a badge; its tooltip explains, and clicking selects the model.
  const close = await point(r => r.kind === 'layer' && r.layer === model, () => 87);
  await page.mouse.click(close.x, close.y);
  await expect.poll(async () => (await rows()).rows.some((r: any) => r.kind === 'material')).toBe(false);
  await page.evaluate(() => (window as any).PM.selectLayers(['title']));
  await page.waitForTimeout(200);
  const badge = await page.evaluate(() => {
    const T = (window as any).PM.Kernel.services.get('timeline');
    const i = T.rows.findIndex((r: any) => r.kind === 'layer' && r.L.id === 'title');
    return { i };
  });
  expect(badge.i).toBeGreaterThanOrEqual(0);
  const state = await rows();
  const box = await cvBox();
  const y = box.y + state.ruler + badge.i * state.row - state.scrollY + state.row / 2;
  // Find the badge by its tooltip along the row.
  const tip = () => page.evaluate(() => { const T = (window as any).PM.Kernel.services.get('timeline').cv; return (T.title || T.dataset.tipTitle || '') as string; });
  let found: number | null = null;
  for (let x = 160; x < 900 && found == null; x += 3) {
    await page.mouse.move(box.x + x, y);
    const title = await tip();
    if (title?.startsWith('Used as a material — on phone')) found = x;
  }
  expect(found).not.toBeNull();
  expect(await tip()).toContain('hidden layers still work as materials');
  await page.waitForTimeout(600);
  await shot(page, '02-after-15-used-as-material-badge-hover');
  await page.mouse.click(box.x + found!, y);
  await expect.poll(() => page.evaluate(() => (window as any).PM.sel.layers)).toEqual([model]);
  await shot(page, '02-after-16-badge-click-selects-model');

  await page.evaluate(() => (window as any).PM.theme.apply('dark'));
  await shot(page, '02-after-17-timeline-dark');
  expect(session.diagnostics.pageErrors).toEqual([]);
});
