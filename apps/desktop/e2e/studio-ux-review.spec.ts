import { expect, test } from './helpers/app';

/* The 3D production UX review: Render Quality in the composition and in
   Export, the viewer's Refine indicator, layers that surround the scene,
   side-view feedback, plain-language errors and the 3D empty state. */

const quality = (page: any) => page.evaluate(() => (window as any).PM.Kernel.api('ux-e2e').scene3d.renderQuality.get());
async function pick(page: any, field: any, option: string) {
  await field.scrollIntoViewIfNeeded().catch(() => {});
  await field.locator('xpath=..').click();
  await page.locator('.pm-menu [role="option"], .pm-menu [role="menuitemradio"], .pm-menu .di').filter({ hasText: option }).first().click();
}

test('Render Quality: presets with a plain estimate, Custom settings, one Undo each, and the camera points to it', async ({ session }) => {
  test.setTimeout(120_000);
  await session.openEditor();
  const page = session.page;
  // The 3D Layer entry in an empty composition opens the Add 3D Layer menu.
  await page.locator('.tl-empty-button').filter({ hasText: '3D Layer' }).click();
  await expect(page.locator('.menu-panel .di, .pm-menu .di').filter({ hasText: 'Fog Volume' }).first()).toBeVisible();
  await page.keyboard.press('Escape');
  const camera = await page.evaluate(() => {
    const PM = (window as any).PM; PM.pause(); PM.AutomaticPreview?.cancel?.();
    const api = PM.Kernel.api('ux-e2e').scene3d;
    const id = api.edit({ operation: 'add_camera', camera: { p: { x: 0, y: 1, z: 5 } } }).data.result.id;
    api.edit({ operation: 'add_object', object: { name: 'Cube', source: { primitive: 'box' } } });
    PM.selectLayers([]);
    return id;
  });
  const inspector = page.locator('#panel-inspector');
  const about = inspector.locator('[data-render-quality-about]');
  await expect(about).toContainText('Up to about 3 s per frame');
  const field = inspector.getByLabel('Quality', { exact: true }).first();
  await pick(page, field, 'Draft');
  await expect.poll(async () => (await quality(page)).preset).toBe('draft');
  await expect(about).toContainText('Fast and grainy');
  expect(await page.evaluate(() => (window as any).PM.hist.label())).toBe('Render Quality');

  // Custom reveals the advanced settings, starting from the current values.
  await expect(inspector.getByRole('group', { name: 'Custom render quality' })).toHaveCount(0);
  await pick(page, field, 'Custom');
  const custom = inspector.getByRole('group', { name: 'Custom render quality' });
  await expect(custom).toBeVisible();
  await expect.poll(async () => (await quality(page)).maxSamples).toBe(16);
  const max = custom.getByLabel('Max Samples', { exact: true }).first();
  await max.click(); await max.fill('48'); await page.keyboard.press('Enter');
  await expect.poll(async () => (await quality(page)).maxSamples).toBe(48);
  expect(await page.evaluate(() => (window as any).PM.hist.label())).toBe('Max Samples');
  await page.evaluate(() => (window as any).PM.hist.undo());
  await expect.poll(async () => (await quality(page)).maxSamples).toBe(16);

  // The camera no longer has its own sample picker: it names the composition's quality.
  await page.evaluate((id: string) => (window as any).PM.selectLayers([id]), camera);
  const link = inspector.locator('[data-render-quality-link]');
  await expect(link).toHaveText('Custom');
  await link.click();
  await expect(about).toBeVisible();
  // The Refine indicator works toward the same quality and says where it is set.
  await expect(page.locator('#studio-refine')).toHaveAttribute('title', /Custom quality.*Render Quality/, { timeout: 20_000 });
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('Export: a per-export Render Quality override with a whole-export estimate; passes explain how to get them', async ({ session }) => {
  test.setTimeout(90_000);
  await session.openEditor();
  const page = session.page;
  await page.evaluate(() => {
    const PM = (window as any).PM; PM.pause();
    PM.Kernel.api('ux-e2e').scene3d.edit({ operation: 'add_object', object: { name: 'Cube', source: { primitive: 'box' } } });
    PM.Export.dialog();
  });
  const dialog = page.getByRole('dialog', { name: 'Export', exact: true });
  const select = dialog.locator('select[aria-label="3D Render Quality"]');
  await expect(select).toHaveValue('__composition__');
  const row = dialog.locator('.settings-row').filter({ hasText: '3D Render Quality' });
  await expect(row).toContainText('Saved with this composition.');
  await select.selectOption('final', { force: true });
  await expect(row).toContainText('This export only.');
  await expect(row).toContainText(/Up to about .* for \d+ frames/);
  await expect(dialog.locator('.settings-row:has(b:text-is("Max Samples"))')).toBeHidden();
  await select.selectOption('custom', { force: true });
  await expect(dialog.locator('.settings-row:has(b:text-is("Max Samples"))')).toBeVisible();
  // The override never changes the composition.
  expect(await page.evaluate(() => (window as any).PM.Kernel.api('ux-e2e').scene3d.renderQuality.get().preset)).toBe('production');
  // Video: passes are shown but unavailable, with how to get them.
  const passes = dialog.locator('.settings-row').filter({ hasText: 'Render Passes' });
  await expect(passes).toContainText('Choose Images, then PNG sequence');
  await expect(passes.locator('input').first()).toBeDisabled();
  await dialog.getByRole('button', { name: 'Cancel' }).click();
});

test('World, Ambient and Environment lights: no Link Layers or Move tools; broken models and missing fonts explain themselves', async ({ session }) => {
  test.setTimeout(120_000);
  await session.openEditor();
  const page = session.page;
  const ids = await page.evaluate(() => {
    const PM = (window as any).PM; PM.pause();
    const api = PM.Kernel.api('ux-e2e').scene3d;
    const env = api.edit({ operation: 'add_light', light: { type: 'environment' } }).data.result.id;
    const spot = api.edit({ operation: 'add_light', light: { type: 'spot' } }).data.result.id;
    return { env, spot };
  });
  const inspector = page.locator('#panel-inspector');
  await page.evaluate((id: string) => (window as any).PM.selectLayers([id]), ids.spot);
  await expect(inspector.getByText('Link Layers', { exact: true })).toBeVisible();
  await page.evaluate((id: string) => (window as any).PM.selectLayers([id]), ids.env);
  await expect(inspector.getByText('Link Layers', { exact: true })).toHaveCount(0);
  await expect(page.locator('#composition-3d-controls')).toContainText('no position');
  await expect(page.locator('#composition-3d-controls [role=tab]').filter({ hasText: 'Rotate' })).toHaveCount(0);
  const menu = await page.evaluate((id: string) => { const PM = (window as any).PM; return PM.layerContextItems?.([id])?.map((i: any) => i.label) ?? null; }, ids.env);
  if (menu) expect(menu).not.toContain('Link Layers');

  // A damaged model names the file and says what is wrong, from the app.
  const result = await page.evaluate(async () => (window as any).PM.cmd('3d.import-obj', new File([new Uint8Array([1, 2, 3, 4])], 'Broken.glb')));
  expect(result.message).toBe('Couldn’t import Broken.glb. The file isn’t a readable GLB model — it may be damaged or incomplete.');
  const toast = page.locator('.toast').filter({ hasText: 'Couldn’t import Broken.glb' });
  await expect(toast).toBeVisible();
  await expect(toast).not.toContainText('layers-3d');

  // Extruded text in a font that isn't installed says so instead of vanishing.
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.Edit.apply({ type: 'add_layer', id: 'missing-font', layerType: 'text', name: 'Missing', content: { text: 'Missing', font: 'No Such Font Pro', size: 160 } });
    PM.Kernel.api('ux-e2e').scene3d.setExtrusion('missing-font', { enabled: true, depth: 40 });
    PM.selectLayers(['missing-font']);
  });
  await expect(inspector.locator('[data-extrusion-font-problem]')).toContainText('“No Such Font Pro” isn’t installed on this Mac', { timeout: 20_000 });
});

test('2 Views: the side view says what it shows', async ({ session }) => {
  test.setTimeout(90_000);
  await session.openEditor();
  const page = session.page;
  await page.evaluate(() => {
    const PM = (window as any).PM; PM.pause();
    const api = PM.Kernel.api('ux-e2e').scene3d;
    api.edit({ operation: 'add_camera', camera: { p: { x: 4, y: 3, z: 6, targetX: 0, targetY: 0, targetZ: 0 } } });
    api.edit({ operation: 'add_object', object: { name: 'Box', source: { primitive: 'box' } } });
    api.views.setLayout(2);
  });
  const status = page.locator('.viewer-pane[data-pane="0"] .viewer-pane-status');
  await expect(status).toBeVisible();
  await expect(status).toHaveText('Live', { timeout: 20_000 });
  await expect(status).toHaveAttribute('title', /Live quality/);
});
