import { expect, test } from './helpers/app';
/* The 3D toolbar's light tool: Add, then click a model, puts a softbox whose
   highlight lands there; dragging slides it; the panel's sliders tune it live
   and one Undo takes a slider back; Esc leaves the tool. Fast preview draws
   with the real-time renderer and back. */
test('the light tool adds, aims and tunes a light; Fast preview switches renderers', async ({ session }) => {
  test.setTimeout(180_000);
  await session.openEditor();
  const { page } = session;
  await page.evaluate(() => {
    const PM = (window as any).PM, api = PM.Kernel.api('light-tool');
    api.scene3d.edit({ operation: 'add_camera', camera: { p: { x: 0, y: 0.8, z: 5, targetX: 0, targetY: 0, targetZ: 0 } } });
    api.scene3d.edit({ operation: 'add_object', object: { id: 'cube', source: { primitive: 'roundedBox' }, material: { p: { color: '#C0C4CA', metalness: 1, roughness: 0.3 } } } });
    api.scene3d.edit({ operation: 'auto_studio' });
    PM.invalidate();
  });
  await page.waitForSelector('#composition-3d-controls', { state: 'visible', timeout: 20_000 });
  await page.waitForTimeout(2500);
  // Screenshots for review go outside the repository when PM_SHOTS names a folder.
  const shot = async (name: string) => {
    if (!process.env.PM_SHOTS) return;
    const box = await page.locator('#composition-3d-controls').evaluate(el => el.closest('.panel')?.getBoundingClientRect().toJSON() ?? null);
    await page.screenshot({ path: `${process.env.PM_SHOTS}/${name}.png`, clip: box ? { x: box.x, y: box.y, width: box.width, height: Math.min(box.height, 520) } : undefined });
  };
  const values = () => page.evaluate(() => [...document.querySelectorAll('#light-panel .lp-row:not([hidden]) .lp-val')].map(e => e.textContent).join(' | '));
  await shot('after-1-default');
  await page.locator('#composition-3d-controls button[aria-label="Light"]').click();
  const stage = await page.evaluate(() => { const r = document.querySelector('#composition-3d-controls')!.closest('.panel')!.getBoundingClientRect(); return { x: r.x, y: r.y, w: r.width, h: r.height }; });
  const c = { x: stage.x + stage.w / 2, y: stage.y + stage.h / 2 - 20 };
  await page.mouse.move(c.x, c.y); await page.waitForTimeout(400);
  await shot('after-2-light-on');
  // Add, then click a model: a new softbox, its highlight where we clicked; dragging slides it.
  await page.locator('#light-panel button', { hasText: 'Add' }).click();
  await page.mouse.move(c.x, c.y); await page.waitForTimeout(300);
  await shot('after-3-adding');
  const before = await page.evaluate(() => (window as any).PM.proj.layers.filter((l: any) => l.d?.data?.light).length);
  await page.mouse.down(); await page.mouse.move(c.x + 60, c.y - 30, { steps: 8 }); await page.mouse.up();
  await page.waitForTimeout(800);
  const after = await page.evaluate(() => (window as any).PM.proj.layers.filter((l: any) => l.d?.data?.light).length);
  const panelBefore = await values();
  // Brightness slider: drag it right.
  const slider = page.locator('#light-panel .lp-row:not([hidden]) input[type=range]').first();
  const sb = (await slider.boundingBox())!;
  await page.mouse.move(sb.x + sb.width * 0.45, sb.y + sb.height / 2); await page.mouse.down();
  await page.mouse.move(sb.x + sb.width * 0.7, sb.y + sb.height / 2, { steps: 6 }); await page.mouse.up();
  await page.waitForTimeout(600);
  const panelAfter = await values();
  await page.mouse.move(c.x + 60, c.y - 30);
  await shot('after-4-light-placed');
  const info = await page.evaluate(() => (window as any).PM.Kernel.api('light-tool').scene3d.lights.info());
  // One undo takes back the slider; the panel follows.
  await page.evaluate(() => (window as any).PM.hist.undo());
  await page.waitForTimeout(500);
  const panelUndone = await values();
  await page.locator('#composition-3d-controls button', { hasText: 'Fast' }).click();
  await page.waitForTimeout(800);
  await shot('after-5-fast');
  await page.keyboard.press('Escape');
  await page.waitForTimeout(300);
  const toolOff = await page.evaluate(() => !(window as any).PM.Kernel.api('light-tool').scene3d.lights.getTool().on);
  await page.locator('#composition-3d-controls button', { hasText: 'Quality' }).click();
  const preview = await page.evaluate(() => (window as any).PM.Kernel.api('light-tool').scene3d.studio.getPreview());
  expect(after).toBe(before + 1);
  expect(panelAfter).not.toBe(panelBefore);
  expect(panelUndone).toBe(panelBefore);
  expect(toolOff).toBe(true);
  expect(info?.type).toBe('area');
  expect(preview).toBe('quality');
  expect(session.diagnostics.pageErrors).toEqual([]);
});
