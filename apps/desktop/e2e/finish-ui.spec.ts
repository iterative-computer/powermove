import { expect, test } from './helpers/app';
import { twoMaterialGlb } from './helpers/two-material-glb';

/* The finishing UI pass over the engine gaps: workspace Reset state, undoable
   preset delete/rename, material thumbnails, dragging a Timeline layer onto a
   material slot, End offset for every exit animator, ⌘ to scale a shape's
   handles, and Pre-compose remembering its options. */

const MENU = '[data-svelte-overlay-menu] .menu-panel';

test('workspace Reset is live from the first edit of a built-in, and reads Unchanged after', async ({ session }) => {
  const { page } = session;
  await session.openEditor();
  await page.evaluate(() => { const PM = (window as any).PM; PM.WS.reset('design'); PM.WS.activate('design', true); });
  const more = page.locator('.tb-workspaces-more');
  const resetRow = () => page.locator(`${MENU} .di`).filter({ hasText: 'Reset to Saved Layout' });

  await page.evaluate(() => (window as any).PM.WS.mutate((ws: any) => { ws.layout.docks.find((d: any) => d.id === 'left').size = 377; }));
  expect(await page.evaluate(() => (window as any).PM.WS.current.name)).toBe('Default (edited)');
  await page.evaluate(() => { (window as any).PM.Kernel.api('finish-ui-e2e').panels.open('workspaces'); });
  const panel = page.locator('[data-svelte-panel="workspaces"]');
  await expect(panel.locator('.lyr', { hasText: 'Default (edited)' }).locator('.ws-edited')).toHaveText('Edited');

  await more.click();
  await expect(resetRow()).not.toHaveAttribute('aria-disabled', 'true');
  await resetRow().click();
  await expect.poll(() => page.evaluate(() => (window as any).PM.WS.current.layout.docks.find((d: any) => d.id === 'left').size)).not.toBe(377);
  await more.click();
  await expect(resetRow().locator('.kb')).toHaveText('Unchanged');
  await page.evaluate(() => (window as any).PM.closeMenus());
  await expect(panel.locator('.ws-edited')).toHaveCount(0);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('My Presets delete at once with Undo in the toast, and rename can be undone', async ({ session }) => {
  const { page } = session;
  await session.openEditor();
  await page.evaluate(() => {
    const PM = (window as any).PM;
    const layer = PM.mkLayer('shape', { name: 'Card', d: { shape: 'rect', w: 400, h: 300, color: '#f4dfb0', stroke: 0 } });
    PM.proj.layers = [layer]; PM.ProjectIndex.invalidate();
    PM.animate(layer, 'opacity', [{ t: 0, v: 0 }, { t: 1, v: 100 }]); PM.touch(); PM.selectLayers(layer.id);
    for (const name of ['Soft entrance', 'Quick fade', 'Glow pulse']) PM.Presets.save({ layerId: layer.id, name, effectIds: [], propertyPaths: ['opacity'] });
    PM.WS.activate('effects', true);
  });
  const browser = page.locator('.fxb');
  const names = () => page.evaluate(() => (window as any).PM.Presets.list().map((p: any) => p.name));
  const row = browser.locator('[data-kind="preset"]').filter({ hasText: 'Quick fade' });
  let confirmed = false;
  await page.exposeFunction('__confirmCalled', () => { confirmed = true; });
  await page.evaluate(() => { const PM = (window as any).PM, real = PM.confirm; PM.confirm = (...args: any[]) => { (window as any).__confirmCalled(); return real(...args); }; });
  await row.locator('.fxb-pick').focus();
  await page.keyboard.press('Backspace');
  await expect(row).toHaveCount(0);
  expect(confirmed).toBe(false);
  expect(await names()).toEqual(['Soft entrance', 'Glow pulse']);
  // Focus moves to the row that took its place.
  await expect(browser.locator('[data-kind="preset"]').filter({ hasText: 'Glow pulse' }).locator('.fxb-pick')).toBeFocused();
  const toast = page.locator('.toast, [data-toast]').filter({ hasText: 'Deleted “Quick fade”' }).first();
  await toast.getByRole('button', { name: 'Undo' }).click();
  await expect.poll(names).toEqual(['Soft entrance', 'Quick fade', 'Glow pulse']);

  await row.locator('.fxb-pick').focus();
  await page.keyboard.press('F2');
  await browser.locator('.fxb-rename').fill('Fast fade');
  await browser.locator('.fxb-rename').press('Enter');
  await expect.poll(names).toEqual(['Soft entrance', 'Fast fade', 'Glow pulse']);
  await page.locator('.toast, [data-toast]').filter({ hasText: 'Renamed “Quick fade” to “Fast fade”' }).first().getByRole('button', { name: 'Undo' }).click();
  await expect.poll(names).toEqual(['Soft entrance', 'Quick fade', 'Glow pulse']);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

async function materialsScene(session: any) {
  await session.openEditor();
  const page = session.page;
  await page.setViewportSize({ width: 1440, height: 900 });
  return page.evaluate(async (bytes: number[]) => {
    const PM = (window as any).PM; PM.pause(); PM.AutomaticPreview?.cancel?.();
    const api = PM.Kernel.api('finish-ui-e2e');
    api.scene3d.edit({ operation: 'add_camera', camera: { p: { x: 0, y: 0, z: 2.6 } } });
    PM.Edit.apply({ type: 'add_layer', id: 'paint', layerType: 'solid', name: 'Red Solid', visible: false, content: { w: 256, h: 256, color: '#E5484D' } });
    PM.Edit.apply({ type: 'add_layer', id: 'title', layerType: 'text', name: 'Title', visible: false, content: { text: 'HELLO', size: 64, color: '#FFFFFF' } });
    PM.Edit.apply({ type: 'add_layer', id: 'tone', layerType: 'audio', name: 'Music' });
    const imported = await PM.cmd('3d.import-obj', new File([new Uint8Array(bytes)], 'phone.glb', { type: 'model/gltf-binary' }));
    if (!imported.ok) throw new Error(imported.message);
    const model = PM.proj.layers.find((l: any) => l.d?.data?.object).id;
    api.scene3d.setMaterialMap(model, 0, 'color', { layerId: 'paint' });
    PM.selectLayers([model]);
    const panel = document.getElementById('panel-timeline'); if (panel) panel.style.flex = '0 0 420px';
    window.dispatchEvent(new Event('resize'));
    return { model };
  }, [...twoMaterialGlb()]);
}
const maps = (page: any, id: string) => page.evaluate((id: string) => (window as any).PM.L(id).d.data.object.sourceMaterials ?? {}, id);

test('material sources show their cached pictures in the slot button and the picker', async ({ session }) => {
  test.setTimeout(90_000);
  const page = session.page;
  await materialsScene(session);
  const section = page.locator('[data-materials-section]');
  await section.locator('[data-material-index="0"] .mat-twirl').click();
  const slot = section.locator('[data-material-slot="0:color"]');
  await slot.scrollIntoViewIfNeeded();
  // Placeholder first, then the picture, in the same 16px box.
  const thumb = slot.locator('[data-material-thumb]');
  const before = await thumb.boundingBox();
  await expect(thumb).toHaveAttribute('data-material-thumb', 'ready', { timeout: 20_000 });
  expect(await thumb.boundingBox()).toEqual(before);
  expect(await thumb.locator('img').getAttribute('src')).toMatch(/^data:image\/png/);
  await slot.locator('button').click();
  const picker = page.locator('[data-material-picker="color"]');
  await expect(picker.locator('[data-option="layer:paint"] [data-material-thumb]')).toHaveAttribute('data-material-thumb', 'ready', { timeout: 20_000 });
  // Sources that can't be used keep their icon.
  await expect(picker.locator('[data-option="layer:tone"] [data-material-thumb]')).toHaveCount(0);
  await page.keyboard.press('Escape');
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('dragging a Timeline layer onto a material slot assigns it in one undo step, and reorder still works', async ({ session }) => {
  test.setTimeout(120_000);
  const page = session.page;
  const { model } = await materialsScene(session);
  const section = page.locator('[data-materials-section]');
  await section.locator('[data-material-index="1"] .mat-twirl').click();
  const bodySlot = section.locator('[data-material-slot="1:color"]');
  await bodySlot.scrollIntoViewIfNeeded();
  const rowPoint = (id: string) => page.evaluate((id: string) => {
    const tl = (window as any).PM.Kernel.services.get('timeline');
    const index = tl.rows.findIndex((r: any) => r.kind === 'layer' && r.L.id === id);
    const rect = tl.cv.getBoundingClientRect();
    return { x: rect.left + 190, y: rect.top + tl.ruler + index * tl.row - tl.scrollY + tl.row / 2 };
  }, id);
  const slotPoint = async () => { const box = (await bodySlot.boundingBox())!; return { x: box.x + box.width * .75, y: box.y + box.height / 2 }; };
  const drag = async (from: { x: number; y: number }, to: { x: number; y: number }, during?: () => Promise<void>) => {
    await page.mouse.move(from.x, from.y); await page.mouse.down();
    await page.mouse.move(from.x + 20, from.y - 8, { steps: 3 });
    await page.mouse.move(to.x, to.y, { steps: 10 });
    await during?.();
    await page.mouse.up();
  };
  const historyLength = () => page.evaluate(() => (window as any).PM.hist.list().length);

  // An audio layer is refused with its reason; nothing changes, the model stays selected.
  await drag(await rowPoint('tone'), await slotPoint(), async () => {
    await expect(bodySlot).toHaveAttribute('data-layer-drop', 'invalid');
    await expect(page.locator('.pm-layer-drag-badge')).toHaveText('Audio has no picture');
  });
  expect((await maps(page, model))[1]?.color).toBeUndefined();
  expect(await page.evaluate(() => (window as any).PM.sel.layers)).toEqual([model]);
  await expect(page.locator('.pm-layer-drag-badge')).toHaveCount(0);

  // A text layer fits: highlighted, then assigned in one step.
  const steps = await historyLength();
  await drag(await rowPoint('title'), await slotPoint(), async () => {
    await expect(bodySlot).toHaveAttribute('data-layer-drop', 'valid');
    await expect(page.locator('.pm-layer-drag-badge')).toHaveText('Use “Title” as Base Color');
  });
  await expect.poll(async () => (await maps(page, model))[1]?.color).toEqual({ layerId: 'title' });
  expect(await historyLength()).toBe(steps + 1);
  expect(await page.evaluate(() => (window as any).PM.sel.layers)).toEqual([model]);
  await page.evaluate(() => (window as any).PM.hist.undo());
  await expect.poll(async () => (await maps(page, model))[1]?.color).toBeUndefined();

  // Onto the Timeline's own Materials row (Whole Model Base Color).
  await page.evaluate((id: string) => { const PM = (window as any).PM; PM.Kernel.api('finish-ui-e2e').uiState.setLayerCollapsed(PM.L(id), false); PM.invalidate(); }, model);
  await page.waitForTimeout(300);
  const materialRow = await page.evaluate(() => {
    const tl = (window as any).PM.Kernel.services.get('timeline');
    const i = tl.rows.findIndex((r: any) => r.kind === 'material' && r.slot === 'color' && r.index === null);
    const rect = tl.cv.getBoundingClientRect();
    return { x: rect.left + tl.gut - 60, y: rect.top + tl.ruler + i * tl.row - tl.scrollY + tl.row / 2 };
  });
  await drag(await rowPoint('paint'), materialRow, async () => {
    await expect.poll(() => page.evaluate(() => (window as any).PM.Kernel.services.get('timeline').materialDrop?.ok)).toBe(true);
  });
  await expect.poll(() => page.evaluate((id: string) => (window as any).PM.L(id).d.data.object.material?.maps?.color, model)).toEqual({ layerId: 'paint' });

  // A plain reorder still works and selects the moved layer.
  await page.evaluate(() => (window as any).PM.selectLayers([]));
  const order = () => page.evaluate(() => (window as any).PM.proj.layers.map((l: any) => l.id));
  const start = await order();
  const titleRow = await rowPoint('title');
  const toneRow = await rowPoint('tone');
  await drag(titleRow, { x: toneRow.x, y: toneRow.y - 8 });
  await expect.poll(order).not.toEqual(start);
  expect(await page.evaluate(() => (window as any).PM.sel.layers)).toEqual(['title']);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('every exit animator shows and edits End offset, keeping a migrated channel’s keys', async ({ session }) => {
  const { page } = session;
  await session.openEditor();
  await page.evaluate(() => {
    const PM = (window as any).PM;
    const P = (v: number) => PM.P(v);
    const layer = PM.mkLayer('text', { name: 'Title', from: 0, dur: 6, d: { text: 'HELLO', size: 120, animators: [{
      id: 'ta1', name: 'Fade Out', mode: 'stagger', unit: 'characters', direction: 'out', easing: 'linear', timingVersion: 2, legacyStartDuration: 6,
      p: { delay: P(3.2), duration: P(1), stagger: P(.1), opacity: P(0) } }] } });
    PM.proj.layers = [layer]; PM.ProjectIndex.invalidate(); PM.touch(); PM.selectLayers(layer.id); PM.setTime(1, { raw: true, force: true });
    layer.d.animators[0].p.delay.kf = [PM.KF(0, 3.2), PM.KF(4, 3.2)]; PM.touch();
  });
  const row = page.locator('#panel-inspector [data-channel="ta.ta1.delay"]');
  await row.scrollIntoViewIfNeeded();
  await expect(row.locator('.k')).toHaveText('End offset');
  // 6 s − 3.2 s start − (1 s + 4 × 0.1 s) of stagger.
  await expect(row.locator('input.num')).toHaveValue(/^1\.4/);
  const input = row.locator('input.num');
  await input.click(); await input.fill('0.5'); await input.press('Enter');
  const delay = await page.evaluate(() => (window as any).PM.L((window as any).PM.proj.layers[0].id).d.animators[0].p.delay);
  // Animated: a key lands at the playhead on the original channel; the others stay.
  expect(delay.kf.map((k: any) => [k.t, Number(k.v.toFixed(3))])).toEqual([[0, 3.2], [1, 4.1], [4, 3.2]]);
  // End offset 0.5 → start 6 − 0.5 − 1.4 = 4.1 at the playhead's key.
  expect(await page.evaluate(() => { const PM = (window as any).PM, L = PM.proj.layers[0]; return PM.evP(L, L.d.animators[0].p.delay, 1, 'ta.ta1.delay'); })).toBeCloseTo(4.1, 2);
  await expect(row.locator('input.num')).toHaveValue(/^0\.5/);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('holding ⌘ scales a shape from its handles instead of changing Size, live', async ({ session }) => {
  const { page } = session;
  await session.openEditor();
  const id = await page.evaluate(async () => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ name: 'Shapes', w: 960, h: 540, fps: 30, dur: 6 });
    const card = PM.mkLayer('shape', { name: 'Card', d: { shape: 'rect', w: 260, h: 160, radius: 28, color: '#e8e2cf', stroke: 0 }, p: { 'position.x': 420, 'position.y': 250 } }, project);
    project.layers = [card]; PM.replaceProject(project); PM.setTime(2, { raw: true, force: true }); PM.selectLayers([card.id]);
    const viewer = PM.Kernel.services.get('viewer'); PM.Kernel.services.get('tool').setTool('select');
    viewer.fit = true; viewer.pan = [0, 0]; viewer.layout();
    await new Promise<void>((r) => requestAnimationFrame(() => requestAnimationFrame(() => r())));
    return card.id;
  });
  const corner = await page.evaluate((id: string) => {
    const PM = (window as any).PM, viewer = PM.Kernel.services.get('viewer');
    const L = PM.L(id), T = PM.time; PM.beginEval(T);
    const b = PM.GL.bounds(L, T), m = PM.worldMatrix(L, T), x = b.x0 + b.w, y = b.y0 + b.h;
    return { x: m[0] * x + m[2] * y + m[4], y: m[1] * x + m[3] * y + m[5], shown: viewer.shown };
  }, id);
  const box = (await page.locator('#stage-inner').boundingBox())!;
  const start = { x: box.x + corner.x * corner.shown, y: box.y + corner.y * corner.shown };
  const state = () => page.evaluate((id: string) => {
    const PM = (window as any).PM, L = PM.L(id), c = PM.resolveContent(L, PM.time);
    return { w: c.w, h: c.h, sx: L.p['scale.x'].v, sy: L.p['scale.y'].v };
  }, id);
  const viewer = () => page.evaluate(() => { const v = (window as any).PM.Kernel.services.get('viewer'); return { size: v.sizeReadout ?? null, scale: v.scaleReadout ?? null }; });

  await page.mouse.move(start.x, start.y); await page.mouse.down();
  await page.mouse.move(start.x + 40, start.y + 30, { steps: 5 });
  expect((await viewer()).size).not.toBeNull();
  // Press ⌘ mid-drag without moving: the gesture switches to Scale from its start.
  await page.keyboard.down('Meta');
  await expect.poll(async () => (await viewer()).scale).not.toBeNull();
  expect(await state()).toMatchObject({ w: 260, h: 160 });
  await page.mouse.move(start.x + 60, start.y + 40, { steps: 4 });
  await page.mouse.up();
  await page.keyboard.up('Meta');
  const scaled = await state();
  expect(scaled).toMatchObject({ w: 260, h: 160 });
  expect(scaled.sx).toBeGreaterThan(100); expect(scaled.sy).toBeGreaterThan(100);
  expect(await viewer()).toEqual({ size: null, scale: null });
  expect(await page.evaluate(() => (window as any).PM.hist.list().at(-1))).toBe('Scale layer');
  expect(await page.evaluate(() => (window as any).PM.hist.undo())).toBe(true);
  expect(await state()).toMatchObject({ w: 260, h: 160, sx: 100, sy: 100 });
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('Pre-compose opens with the options used last time', async ({ session }) => {
  const { page } = session;
  await session.openEditor();
  const ids = await page.evaluate(() => {
    const PM = (window as any).PM;
    const mk = (name: string, x: number) => PM.mkLayer('shape', { name, d: { shape: 'ellipse', w: 90, h: 90, color: '#ff9f6b', stroke: 0 }, p: { 'position.x': x, 'position.y': 300 } });
    const layers = [mk('A', 300), mk('B', 600), mk('C', 900)];
    PM.proj.layers = layers; PM.ProjectIndex.invalidate(); PM.touch();
    return layers.map((l: any) => l.id);
  });
  const open = (sel: string[]) => page.evaluate((sel: string[]) => { const PM = (window as any).PM; PM.selectLayers(sel); PM.cmd('precompose'); }, sel);
  const toggle = (modal: any, label: string) => modal.locator('.new-project-field').filter({ hasText: label });
  await open([ids[0]!, ids[1]!]);
  let modal = page.locator('.modal').last();
  for (const label of ['Fit duration to layers', 'Fit size to layers', 'Open new composition']) await toggle(modal, label).getByRole('radio', { name: 'On' }).click();
  await modal.getByRole('button', { name: 'Pre-compose', exact: true }).click();
  await expect(modal).toBeHidden();
  await page.evaluate((id: string) => { const PM = (window as any).PM; const main = PM.Comps.list().find((c: any) => PM.Comps.get(c.id)?.layers?.some((l: any) => l.id === id)); if (main) PM.Comps.open(main.id); }, ids[2]!);
  await open([ids[2]!]);
  modal = page.locator('.modal').last();
  for (const label of ['Fit duration to layers', 'Fit size to layers', 'Open new composition']) {
    await expect(toggle(modal, label).getByRole('radio', { name: 'On' })).toHaveAttribute('aria-checked', 'true');
  }
  await expect(page.locator('.toast, [data-toast]').filter({ hasText: 'precomposeOptions' })).toHaveCount(0);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
