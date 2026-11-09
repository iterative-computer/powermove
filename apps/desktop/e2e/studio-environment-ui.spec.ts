import { mkdir } from 'node:fs/promises';
import { chooseNativeMenu, expect, test } from './helpers/app';

/* World, Terrain, Scatter, Fog Volume, Ambient/Environment lights, Material
   Options, camera iris and render passes through the interface.
   Set PM_SHOT_DIR to also write the review screenshots. */

const shots = process.env.PM_SHOT_DIR;
async function shot(page: any, name: string) {
  if (!shots) return;
  await mkdir(shots, { recursive: true });
  await page.waitForTimeout(2500);
  await page.screenshot({ path: `${shots}/after-${name}.png` });
}
const objects = (page: any) => page.evaluate(() => (window as any).PM.proj.layers.map((l: any) => ({ id: l.id, name: l.name, object: l.d?.data?.object, light: l.d?.data?.light, camera: l.d?.data?.camera })));
async function pick(page: any, field: string, option: string) {
  const select = page.getByLabel(field, { exact: true }).first();
  await select.scrollIntoViewIfNeeded().catch(() => {});
  await select.locator('xpath=..').click();
  await page.locator('.pm-menu [role="option"], .pm-menu [role="menuitemradio"], .pm-menu .di, .pm-menu button').filter({ hasText: option }).first().click();
}

test('Environment layers, Material Options, iris and render passes from the 3D UI', async ({ session }) => {
  test.setTimeout(180_000);
  await session.openEditor();
  const page = session.page;
  const ids = await page.evaluate(() => {
    const PM = (window as any).PM; PM.pause(); PM.AutomaticPreview?.cancel?.();
    const api = PM.Kernel.api('environment-ui-e2e');
    const camera = api.scene3d.edit({ operation: 'add_camera', camera: { p: { x: 18, y: 12, z: 23, fov: 45, fStop: 2.8 } } }).data.result.id;
    const cube = api.scene3d.edit({ operation: 'add_object', object: { name: 'Tree', source: { primitive: 'cone', parameters: { radius: 0.4, height: 2, segments: 8 } }, material: { p: { color: '#334A2E', roughness: 0.85 } } } }).data.result.id;
    return { camera, cube };
  });
  const menu = () => page.evaluate(() => (window as any).PM.cmd('3d.add-menu'));

  // World from the Add menu, with a preset picked there.
  if (shots) { await menu(); await shot(page, '01-add-menu'); await page.evaluate(() => (window as any).PM.closeMenus?.()); }
  await chooseNativeMenu(session, ['World', 'Golden hour'], menu);
  let world = (await objects(page)).find((l: any) => l.object?.world);
  expect(world.object.world.preset).toBe('golden-hour');
  const sky = page.getByRole('radiogroup', { name: 'Sky' });
  await sky.scrollIntoViewIfNeeded();
  await expect(sky.getByRole('radio', { name: 'Golden hour' })).toHaveAttribute('aria-checked', 'true');
  await shot(page, '02-world-golden-hour');
  await sky.getByRole('radio', { name: 'Clear noon' }).click();
  await expect.poll(async () => (await objects(page)).find((l: any) => l.object?.world).object.world.preset).toBe('clear-noon');
  await shot(page, '03-world-clear-noon');

  // Dragging the sun on the dome moves azimuth and elevation in one Undo.
  const dome = page.getByRole('slider', { name: 'Sun position' });
  await dome.scrollIntoViewIfNeeded();
  const box = (await dome.boundingBox())!;
  const before = (await objects(page)).find((l: any) => l.object?.world).object.world.p;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width * 0.8, box.y + box.height * 0.45, { steps: 6 });
  await page.mouse.move(box.x + box.width * 0.7, box.y + box.height * 0.32, { steps: 6 });
  await page.mouse.up();
  const after = (await objects(page)).find((l: any) => l.object?.world).object.world.p;
  expect(after.azimuth.v).not.toBe(before.azimuth.v);
  expect(after.elevation.v).toBeLessThan(before.elevation.v);
  await shot(page, '04-sun-dragged');
  await page.evaluate(() => (window as any).PM.hist.undo());
  const undone = (await objects(page)).find((l: any) => l.object?.world).object.world.p;
  expect(undone.azimuth.v).toBe(before.azimuth.v);
  expect(undone.elevation.v).toBe(before.elevation.v);
  await page.evaluate(() => (window as any).PM.hist.redo());

  // The camera inspector shows the same World, so the sky is one click from the shot.
  await page.evaluate((id: string) => (window as any).PM.selectLayers([id]), ids.camera);
  await expect(page.getByRole('radiogroup', { name: 'Sky' })).toBeVisible();
  await pick(page, 'Iris shape', '6 blades');
  await expect.poll(async () => (await objects(page)).find((l: any) => l.id === ids.camera).camera.p.irisShape.v).toBe(6);
  await page.locator('[data-scene-channel="camera.irisShape"]').scrollIntoViewIfNeeded();
  await shot(page, '05-camera-iris-and-world');

  // A Terrain looks like a landscape at once: three ground covers.
  await chooseNativeMenu(session, ['Terrain'], menu);
  const terrain = (await objects(page)).find((l: any) => l.object?.terrain);
  expect(terrain.object.terrain.regions).toHaveLength(3);
  await page.getByRole('button', { name: 'Mountains' }).click();
  await expect.poll(async () => (await objects(page)).find((l: any) => l.object?.terrain).object.terrain.p.ridged.v).toBe(true);
  await shot(page, '06-terrain-mountains');

  // Scatter asks what to copy and what to cover.
  const rows = await chooseNativeMenu(session, ['Scatter', 'Tree', 'Terrain'], menu);
  expect(rows).toContain('Terrain');
  const scatter = (await objects(page)).find((l: any) => l.object?.scatter);
  expect(scatter.object.scatter).toMatchObject({ model: ids.cube, surface: terrain.id });
  // The copied model becomes the pattern: hidden in the same Undo, copies still drawn.
  const shown = (id: string) => page.evaluate((layerId: string) => { const on = (window as any).PM.L(layerId).on; return typeof on === 'object' && on ? on.v : on; }, id);
  expect(await shown(ids.cube)).toBe(false);
  await expect(page.getByLabel('Copies of', { exact: true }).first()).toBeAttached();
  const seed = scatter.object.scatter.p.seed.v;
  await page.getByRole('button', { name: 'New variation' }).click();
  await expect.poll(async () => (await objects(page)).find((l: any) => l.object?.scatter).object.scatter.p.seed.v).toBe(seed + 1);
  await shot(page, '07-scatter');

  // Fog Volume, keyframed with its diamond like any other property.
  await chooseNativeMenu(session, ['Fog Volume'], menu);
  const fogKey = page.locator('[data-scene-channel="fog.density"] .property-stopwatch');
  await fogKey.click();
  await expect.poll(async () => (await objects(page)).find((l: any) => l.object?.fog).object.fog.p.density.kf.length).toBe(1);
  await shot(page, '08-fog-volume');

  // Ambient and Environment lights.
  await chooseNativeMenu(session, ['Ambient Light'], menu);
  await chooseNativeMenu(session, ['Environment Light'], menu);
  const lights = (await objects(page)).filter((l: any) => l.light).map((l: any) => l.light.type);
  expect(lights).toEqual(expect.arrayContaining(['ambient', 'environment']));
  await expect(page.getByRole('radiogroup', { name: 'Sky' })).toBeVisible();
  await page.getByRole('radiogroup', { name: 'Sky' }).getByRole('radio', { name: 'Overcast' }).click();
  await expect.poll(async () => (await objects(page)).find((l: any) => l.light?.type === 'environment').light.world.preset).toBe('overcast');
  await shot(page, '09-environment-light');
  // A surrounding light has no place and no pixels: no Transform, alignment, Effects or Masks,
  // and the Properties header names its kind.
  const panel = page.locator('[data-inspector-layer]');
  await expect(page.locator('[data-inspector-header]')).toContainText('Environment Light');
  await expect(page.locator('[data-inspector-header]')).not.toContainText('3D Model');
  for (const title of ['Transform', 'Effects', 'Masks']) await expect(panel.getByText(title, { exact: true })).toHaveCount(0);

  // Material Options with AE wording.
  await page.evaluate((id: string) => (window as any).PM.selectLayers([id]), ids.cube);
  await pick(page, 'Casts Shadows', 'Only');
  await expect.poll(async () => (await objects(page)).find((l: any) => l.id === ids.cube).object.materialOptions?.p.castsShadows.v).toBe('only');
  await page.locator('[data-scene-channel="materialOptions.castsShadows"]').scrollIntoViewIfNeeded();
  await shot(page, '10-material-options');

  // Render passes in Export ▸ Images ▸ PNG sequence.
  await page.evaluate(() => { const PM = (window as any).PM; PM.Export.dialog(); });
  const dialog = page.getByRole('dialog', { name: 'Export', exact: true });
  await dialog.getByRole('button', { name: 'Images' }).click();
  await dialog.getByRole('combobox', { name: 'Format' }).click();
  await page.getByRole('option', { name: 'PNG sequence' }).click();
  const passes = dialog.getByRole('group', { name: 'Render passes' });
  await expect(passes).toBeVisible();
  await passes.getByLabel('Depth pass').check();
  await passes.getByLabel('Normal pass').check();
  await expect(passes.getByLabel('Depth pass')).toBeChecked();
  await passes.scrollIntoViewIfNeeded();
  await shot(page, '11-export-render-passes');
  await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();

  // Dark theme review of the new sections.
  if (shots) {
    await page.evaluate(() => (window as any).PM.theme.apply('dark'));
    const all = await objects(page);
    for (const [name, find] of [['world', (l: any) => l.object?.world], ['terrain', (l: any) => l.object?.terrain], ['scatter', (l: any) => l.object?.scatter], ['fog', (l: any) => l.object?.fog]] as const) {
      await page.evaluate((id: string) => (window as any).PM.selectLayers([id]), all.find(find).id);
      await shot(page, `12-dark-${name}`);
    }
    await page.evaluate(() => (window as any).PM.theme.apply('light'));
  }
});

test('A Terrain in an empty composition arrives lit and framed, in one Undo', async ({ session }) => {
  test.setTimeout(90_000);
  await session.openEditor();
  const page = session.page;
  await page.evaluate(() => { const PM = (window as any).PM; PM.pause(); });
  await chooseNativeMenu(session, ['Terrain'], () => page.evaluate(() => (window as any).PM.cmd('3d.add-menu')));
  const kinds = async () => (await objects(page)).map((l: any) => l.object?.terrain ? 'terrain' : l.object?.world ? 'world' : l.camera ? 'camera' : 'other').sort();
  expect(await kinds()).toEqual(['camera', 'terrain', 'world']);
  await expect(page.getByRole('button', { name: 'Rolling hills' })).toBeVisible();
  await shot(page, '13-terrain-empty-comp');
  // Selection changes are undoable too; the edit itself is one entry.
  const undo = await page.evaluate(() => { const PM = (window as any).PM; const labels: string[] = []; while (PM.hist.canUndo() && PM.hist.label() !== 'Add Terrain') { labels.push(PM.hist.label()); PM.hist.undo(); } const found = PM.hist.label(); PM.hist.undo(); return { found, skipped: labels }; });
  expect(undo.found).toBe('Add Terrain');
  await expect.poll(async () => (await objects(page)).length).toBe(0);
});

test('The Free View looks from the Front, Top or Right like After Effects 3D views', async ({ session }) => {
  test.setTimeout(90_000);
  await session.openEditor();
  const page = session.page;
  await page.evaluate(() => {
    const PM = (window as any).PM; PM.pause();
    const api = PM.Kernel.api('look-from-e2e');
    api.scene3d.edit({ operation: 'add_camera', camera: { p: { x: 3, y: 2, z: 5 } } });
    api.scene3d.edit({ operation: 'add_object', object: { name: 'Box', source: { primitive: 'box' }, material: { p: { color: '#D9822B' } } } });
    api.scene3d.edit({ operation: 'add_object', object: { name: 'Ball', source: { primitive: 'sphere' }, p: { x: 1.4 }, material: { p: { color: '#2F80ED' } } } });
    api.scene3d.edit({ operation: 'add_light', light: { type: 'area', p: { x: 2, y: 3, z: 3, intensity: 3 } } });
    PM.selectLayers([]);
  });
  const view = () => page.evaluate(() => (window as any).PM.Kernel.api('look-from-e2e').scene3d.getView());
  expect(await view()).toBe('camera');
  for (const side of ['Top', 'Front', 'Right']) {
    const nav = page.getByLabel('3D navigation', { exact: true }).first();
    await nav.locator('xpath=..').click();
    await page.locator('.pm-menu [role="option"], .pm-menu .di, .pm-menu button').filter({ hasText: new RegExp(`^${side}$`) }).first().click();
    expect(await view()).toBe('editor');
    await shot(page, `14-view-${side.toLowerCase()}`);
  }
  await page.getByRole('tab', { name: 'Active Camera', exact: true }).click();
  expect(await view()).toBe('camera');
});

test('A group of models has Exploded view and Turntable in its panel', async ({ session }) => {
  test.setTimeout(90_000);
  await session.openEditor();
  const page = session.page;
  await page.evaluate(() => {
    const PM = (window as any).PM; PM.pause();
    const api = PM.Kernel.api('group-shots-e2e');
    api.scene3d.edit({ operation: 'add_camera', camera: { p: { x: 3, y: 2, z: 6 } } });
    api.scene3d.edit({ operation: 'add_object', object: { id: 'base', source: { primitive: 'box', parameters: { width: 2, height: 0.4, depth: 2 } } } });
    api.scene3d.edit({ operation: 'add_object', object: { id: 'top', source: { primitive: 'sphere', parameters: { radius: 0.3 } }, p: { y: 0.6 } } });
    PM.Edit.apply({ type: 'group_layers', targets: ['base', 'top'], name: 'Product' });
    PM.selectLayers([PM.proj.layers.find((l: any) => l.type === 'group').id]);
  });
  await page.getByRole('button', { name: 'Exploded view' }).click();
  await expect.poll(() => page.evaluate(() => (window as any).PM.L('top').p['position.y'].kf.length)).toBe(2);
  await shot(page, '15-group-exploded-view');
});

test('3D artwork layers have After Effects Material Options', async ({ session }) => {
  test.setTimeout(90_000);
  await session.openEditor();
  const page = session.page;
  await page.evaluate(() => {
    const PM = (window as any).PM; PM.pause();
    PM.Kernel.api('artwork-options-e2e').scene3d.edit({ operation: 'add_camera' });
    PM.Edit.apply({ type: 'add_layer', id: 'card', layerType: 'solid', name: 'Card', threeD: true, content: { w: 400, h: 300, color: '#E5484D' } });
    PM.L('card').threeD = true; PM.touch?.();
    PM.selectLayers(['card']);
  });
  await pick(page, 'Casts Shadows', 'Only');
  await expect.poll(() => page.evaluate(() => (window as any).PM.L('card').d.materialOptions?.p.castsShadows.v)).toBe('only');
  await expect(page.locator('[data-scene-channel="materialOptions.acceptsLights"]')).toBeAttached();
  await page.locator('[data-scene-channel="materialOptions.castsShadows"]').scrollIntoViewIfNeeded();
  await shot(page, '16-artwork-material-options');
});

test('With the World selected, dragging the sky in the viewer moves the sun', async ({ session }) => {
  test.setTimeout(90_000);
  await session.openEditor();
  const page = session.page;
  const world = await page.evaluate(() => {
    const PM = (window as any).PM; PM.pause();
    const api = PM.Kernel.api('sun-drag-e2e');
    api.scene3d.edit({ operation: 'add_camera', camera: { p: { x: 0, y: 1, z: 6, targetY: 1.5 } } });
    const id = api.scene3d.edit({ operation: 'set_world', world: { mode: 'sky', preset: 'clear-noon', p: { azimuth: 0, elevation: 45 } } }).data.result.id;
    PM.selectLayers([id]);
    return id;
  });
  const box = await page.evaluate(() => { const r = document.querySelector('#composition-3d-controls')!.closest('.panel')!.getBoundingClientRect(); return { x: r.x, y: r.y, w: r.width, h: r.height }; });
  const read = () => page.evaluate((id: string) => { const p = (window as any).PM.L(id).d.data.object.world.p; return { azimuth: p.azimuth.v, elevation: p.elevation.v }; }, world);
  await page.mouse.move(box.x + box.w * 0.5, box.y + box.h * 0.3);
  await page.mouse.down();
  await page.mouse.move(box.x + box.w * 0.72, box.y + box.h * 0.38, { steps: 8 });
  await page.mouse.up();
  const after = await read();
  // The camera looks down -Z (azimuth 180); right of centre turns toward +X (below 180).
  expect(after.azimuth).toBeGreaterThan(90);
  expect(after.azimuth).toBeLessThan(175);
  expect(after.elevation).toBeGreaterThan(0);
  expect(after.elevation).toBeLessThan(45);
  expect(await page.evaluate(() => (window as any).PM.hist.label())).toBe('Move sun');
  await shot(page, '17-sun-dragged-in-viewer');
  await page.evaluate(() => (window as any).PM.hist.undo());
  expect(await read()).toEqual({ azimuth: 0, elevation: 45 });
});

test('a World alone has sunlit ground in native and portable Studio', async ({ session }) => {
  test.setTimeout(120_000);
  await session.openEditor();
  await session.page.evaluate(() => {
    const PM = (window as any).PM, api = PM.Kernel.api('world-ground-e2e');
    PM.pause(); PM.AutomaticPreview?.cancel();
    api.scene3d.edit({ operation: 'add_camera', camera: { p: { x: 0, y: 2, z: 5, targetX: 0, targetY: 2, targetZ: 0 } } });
    api.scene3d.edit({ operation: 'set_world', world: { mode: 'sky', preset: 'clear-noon' } });
    PM.invalidate();
  });
  await expect.poll(() => session.page.evaluate(() => (window as any).PM.Studio?.status().ready), { timeout: 30_000 }).toBe(true);
  const result = await session.page.evaluate(async () => {
    const PM = (window as any).PM, api = PM.Kernel.api('world-ground-e2e');
    const W = 320, H = 200;
    const sample = (p: ArrayLike<number>, y: number) => Array.from({ length: 3 }, (_, c) => p[(y * W + W / 2) * 4 + c]!);
    PM.agentFrameCapture = true;
    // Synchronous readback exercises WebGPU, prepared capture exercises the host.
    const portable = PM.GL.renderToPixels(0, W, H, { transparent: false, mblur: false });
    await PM.prepareFrame(0, { mblur: false, width: W, height: H });
    let canvas = PM.renderFrameTo(0, W, H, { mblur: false });
    const native = canvas.getContext('2d').getImageData(0, 0, W, H).data;
    const backend = PM.Studio.status().backend;
    const image = canvas.toDataURL('image/png');
    api.scene3d.edit({ operation: 'set_world', world: { p: { groundColor: '#CC3311' } } });
    await PM.prepareFrame(0, { mblur: false, width: W, height: H });
    canvas = PM.renderFrameTo(0, W, H, { mblur: false });
    const colored = canvas.getContext('2d').getImageData(0, 0, W, H).data;
    PM.agentFrameCapture = false;
    // GL readback is bottom-up; the prepared canvas is top-down.
    return { portable: sample(portable, 10), native: sample(native, H - 11), colored: sample(colored, H - 11), backend, image };
  });
  expect(Math.min(...result.portable)).toBeGreaterThan(100);
  for (let c = 0; c < 3; c++) expect(Math.abs(result.portable[c]! - result.native[c]!)).toBeLessThan(8);
  if (process.platform === 'darwin') expect(result.backend).toBe('native-studio');
  expect(result.colored[0]!).toBeGreaterThan(result.colored[1]! + 40);
  const worldShots = process.env.PM_WORLD_SHOT_DIR ?? shots;
  if (worldShots) {
    await mkdir(worldShots, { recursive: true });
    const { writeFile } = await import('node:fs/promises');
    await writeFile(`${worldShots}/world-only-native.png`, Buffer.from(result.image.split(',')[1]!, 'base64'));
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});
