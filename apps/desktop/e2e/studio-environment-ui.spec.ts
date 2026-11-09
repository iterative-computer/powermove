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
