import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { expect, test } from './helpers/app';

test('background picker keeps every channel above the hex field', async ({ session }, info) => {
  const { page } = session;
  await page.evaluate(() => (window as any).PM.theme.apply('light'));
  await page.evaluate(() => (window as any).PM.newProject());
  await page.getByRole('button', { name: /^Background · / }).click();
  const picker = page.getByRole('dialog', { name: 'Background', exact: true });
  await expect(picker).toBeVisible();
  await picker.screenshot({ path: info.outputPath('background-picker.png') });
  const geometry = await picker.evaluate(el => {
    const hex = el.querySelector('.color-dialog-value')!.getBoundingClientRect();
    const bottom = Math.max(...[...el.querySelectorAll('.color-channel, .color-sv, .color-hue')].map(node => node.getBoundingClientRect().bottom));
    return { hexTop: hex.top, bottom };
  });
  expect(geometry.bottom).toBeLessThanOrEqual(geometry.hexTop - 4);
  const before = await page.getByRole('button', { name: /^Background · / }).getAttribute('aria-label');
  const surface = await picker.getByRole('slider', { name: 'Saturation and brightness' }).boundingBox();
  if (!surface) throw new Error('Missing color surface');
  await page.mouse.click(surface.x + surface.width * .7, surface.y + surface.height * .3);
  const chosen = await picker.getByLabel('Background hex value').inputValue();
  await expect(page.getByRole('button', { name: `Background · ${chosen}` })).toBeVisible();
  await picker.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(page.getByRole('button', { name: before!, exact: true })).toBeVisible();
});

for (const loseCapture of [false, true]) test(`imported image drag commits and undoes${loseCapture ? ' after pointer capture is lost' : ''}`, async ({ session }, info) => {
  await session.openEditor();
  const { page } = session;
  const bytes = await readFile(path.join(__dirname, 'fixtures/still-red.png'));
  await page.evaluate(async bytes => {
    const PM = (window as any).PM;
    PM.replaceProject(PM.mkProject({ name: 'Image movement', w: 1920, h: 1080, dur: 5 }));
    await PM.importFiles([new File([new Uint8Array(bytes)], 'still.png', { type: 'image/png' })]);
    PM.setTime(0);
    PM.Kernel.services.get('tool').setTool('select');
    const viewer = PM.Kernel.services.get('viewer');
    viewer.fit = true; viewer.pan = [0, 0]; viewer.layout();
  }, Array.from(bytes));
  await page.waitForFunction(() => Boolean((window as any).PM.firstSel()));
  const start = await page.evaluate(() => {
    const PM = (window as any).PM, viewer = PM.Kernel.services.get('viewer'), layer = PM.firstSel();
    const rect = viewer.inner.getBoundingClientRect();
    const x = PM.ev(layer, 'position.x', 0), y = PM.ev(layer, 'position.y', 0);
    return { id: layer.id, x, y, sx: rect.left + (x + 20) * viewer.shown, sy: rect.top + (y + 20) * viewer.shown };
  });
  const position = () => page.evaluate(id => {
    const PM = (window as any).PM, layer = PM.L(id);
    return [PM.ev(layer, 'position.x', 0), PM.ev(layer, 'position.y', 0)];
  }, start.id);
  await page.mouse.move(start.sx, start.sy);
  await page.mouse.down();
  await page.mouse.move(start.sx - 75, start.sy - 45, { steps: 12 });
  const moved = await position();
  expect(moved).not.toEqual([start.x, start.y]);
  if (loseCapture) {
    // A rebuilt/reparented canvas can lose capture without cancelling the
    // physical gesture. Window listeners must still own its matching release.
    await page.evaluate(() => {
      const stage = document.getElementById('stage')!;
      stage.releasePointerCapture(1);
    });
    await page.mouse.move(start.sx - 75, start.sy - 45);
    expect(await position()).toEqual(moved);
  }
  await page.mouse.up();
  await page.waitForTimeout(750); // Include deferred rendering and autosave.
  await expect.poll(position).toEqual(moved);
  await page.screenshot({ path: info.outputPath('image-after-drag.png') });
  await page.evaluate(() => (window as any).PM.hist.undo());
  await expect.poll(position).toEqual([start.x, start.y]);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('centered tabs and explicit effect actions keep legacy hidden layers accessible', async ({ session }, info) => {
  await session.openEditor();
  const { page } = session;
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.Edit.apply({ type: 'add_layer', layerType: 'solid', name: 'Feedback layer', shy: true, content: { w: 100, h: 100, color: '#ff6b1a' } });
  });
  for (const width of [1440, 980]) {
    await session.app.evaluate(({ BrowserWindow }, width) => BrowserWindow.getAllWindows()[0]!.setSize(width, 900), width);
    await expect.poll(() => page.evaluate(() => window.innerWidth)).toBe(width);
    const toolbar = await page.locator('#toolbar-strip').boundingBox();
    const viewer = await page.locator('#panel-viewer').boundingBox();
    const titlebar = await page.locator('#titlebar').boundingBox();
    const tabs = await page.locator('#tabs').boundingBox();
    const actions = await page.locator('#tb-right').boundingBox();
    expect(Math.abs(tabs!.x + tabs!.width / 2 - width / 2)).toBeLessThan(1);
    expect(tabs!.x + tabs!.width).toBeLessThan(actions!.x);
    // Tools live on the canvas, below the tab row.
    expect(toolbar!.y).toBeGreaterThanOrEqual(titlebar!.y + titlebar!.height);
    expect(toolbar!.x).toBeGreaterThanOrEqual(viewer!.x);
    expect(toolbar!.x + toolbar!.width).toBeLessThan(viewer!.x + viewer!.width);
  }
  await expect(page.locator('.project-strip-divider')).toHaveCount(0);
  const effect = page.locator('.fxb-row[data-id="blur"]');
  await effect.locator('.fxb-pick').click();
  await effect.locator('.fxb-pick').dblclick();
  const count = () => page.evaluate(() => (window as any).PM.firstSel().fx.length);
  expect(await count()).toBe(0);
  await effect.getByRole('button', { name: 'Add Gaussian Blur', exact: true }).click();
  expect(await count()).toBe(1);
  await effect.getByRole('button', { name: 'Add Gaussian Blur', exact: true }).press('Enter');
  expect(await count()).toBe(2);
  const timelineState = () => page.evaluate(() => {
    const PM = (window as any).PM, layer = PM.firstSel();
    const timeline = PM.Kernel.services.get('timeline');
    return { shy: layer.shy, on: layer.on, row: timeline.rows.some((row: any) => row.kind === 'layer' && row.L.id === layer.id) };
  });
  await expect.poll(timelineState).toEqual({ shy: true, on: true, row: true });
  await expect(page.getByRole('button', { name: 'Hide in timeline', exact: true })).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Shy', exact: true })).toHaveCount(0);
  await page.screenshot({ path: info.outputPath('editor-feedback.png') });
  expect(session.diagnostics.pageErrors).toEqual([]);
});
