import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { expect, test } from './helpers/app';

test('background picker keeps every channel above the hex field', async ({ session }, info) => {
  const { page } = session;
  await page.evaluate(() => (window as any).PM.theme.apply('light'));
  await page.evaluate(() => (window as any).PM.newProject());
  const dialog = page.getByRole('dialog', { name: 'New project', exact: true });
  await page.getByRole('button', { name: /^Background · / }).click();
  const picker = page.getByRole('dialog', { name: 'Background', exact: true });
  await expect(picker).toBeVisible();
  // Measure at rest: the popover scales in from its trigger first.
  await picker.evaluate(element => Promise.all(element.getAnimations().map(animation => animation.finished)));
  await picker.screenshot({ path: info.outputPath('background-picker.png') });
  // The unified picker sets plane | opacity | hue beside the details column,
  // whose OKLCH, HSL and RGB channel rows (wells, letters and copy buttons)
  // stack above the value field.
  const geometry = await picker.evaluate(el => {
    const box = (rect: DOMRect) => ({ left: rect.left, top: rect.top, right: rect.right, bottom: rect.bottom });
    const text = (node: Node) => { const range = document.createRange(); range.selectNodeContents(node); return box(range.getBoundingClientRect()); };
    const rows = [...el.querySelectorAll<HTMLElement>('[role="group"]')].filter(row => row.querySelector('input[type="number"]'));
    return {
      picker: box(el.getBoundingClientRect()),
      hex: box(el.querySelector('[aria-label="Background hex value"]')!.getBoundingClientRect()),
      rows: rows.map(row => row.getAttribute('aria-label')),
      channels: rows.flatMap(row => [
        ...[...row.querySelectorAll('input, button')].map(node => box(node.getBoundingClientRect())),
        ...[...row.querySelectorAll('input + span')].map(text)
      ]),
      inputs: rows.reduce((count, row) => count + row.querySelectorAll('input[type="number"]').length, 0),
      sliders: [...el.querySelectorAll('[role="slider"]')].map(node => ({ name: node.getAttribute('aria-label'), ...box(node.getBoundingClientRect()) }))
    };
  });
  expect(geometry.rows).toEqual(['OKLCH', 'HSL', 'RGB']);
  expect(geometry.inputs).toBe(9);
  // Nine wells, nine letters and three copy buttons.
  expect(geometry.channels).toHaveLength(21);
  for (const channel of geometry.channels) expect(channel.bottom).toBeLessThanOrEqual(geometry.hex.top - 4);
  expect(geometry.sliders.map(slider => slider.name)).toEqual(['Saturation and brightness', 'Opacity', 'Hue']);
  const overlaps = (a: typeof geometry.hex, b: typeof geometry.hex) => a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom;
  for (const [index, slider] of geometry.sliders.entries()) {
    expect(slider.left, slider.name!).toBeGreaterThanOrEqual(geometry.picker.left);
    expect(slider.top, slider.name!).toBeGreaterThanOrEqual(geometry.picker.top);
    expect(slider.right, slider.name!).toBeLessThanOrEqual(geometry.picker.right);
    expect(slider.bottom, slider.name!).toBeLessThanOrEqual(geometry.picker.bottom);
    for (const other of [...geometry.sliders.slice(index + 1), geometry.hex, ...geometry.channels]) {
      expect(overlaps(slider, other), `${slider.name} overlaps ${JSON.stringify(other)}`).toBe(false);
    }
  }
  const before = await page.getByRole('button', { name: /^Background · / }).getAttribute('aria-label');
  const surface = await picker.getByRole('slider', { name: 'Saturation and brightness' }).boundingBox();
  if (!surface) throw new Error('Missing color surface');
  await page.mouse.click(surface.x + surface.width * .7, surface.y + surface.height * .3);
  // The field reads "<colour> / <opacity>%"; the document stores the hex alone.
  const chosen = await picker.getByLabel('Background hex value', { exact: true }).inputValue();
  expect(chosen).toMatch(/^#[0-9A-F]{6} \/ 100%$/);
  const chosenHex = chosen.slice(0, 7);
  expect(`Background · ${chosenHex}`).not.toBe(before);
  await expect(page.getByRole('button', { name: `Background · ${chosenHex}`, exact: true })).toBeVisible();
  // This popover has no Cancel button: Escape cancels the preview, restores
  // the colour it opened with, and stays inside the picker.
  await page.keyboard.press('Escape');
  await expect(picker).toHaveCount(0);
  await expect(page.getByRole('button', { name: before!, exact: true })).toBeVisible();
  await expect(dialog).toBeVisible();
  expect(await page.evaluate(() => (window as any).PM.hist.busy())).toBe(false);
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

test('titlebar layout and explicit effect actions keep legacy hidden layers accessible', async ({ session }, info) => {
  await session.openEditor();
  const { page } = session;
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.Edit.apply({ type: 'add_layer', layerType: 'solid', name: 'Feedback layer', shy: true, content: { w: 100, h: 100, color: '#ff6b1a' } });
  });
  for (const width of [1440, 980]) {
    await session.app.evaluate(({ BrowserWindow }, width) => BrowserWindow.getAllWindows()[0]!.setSize(width, 900), width);
    await expect.poll(() => page.evaluate(() => window.innerWidth)).toBe(width);
    const toolbar = (await page.locator('#toolbar-strip').boundingBox())!;
    const documents = (await page.locator('#tabs').boundingBox())!;
    const settings = (await page.locator('#tb-right [aria-label="Open settings"]').boundingBox())!;
    // The tools lead the right side: after the tabs, before the settings and
    // Export buttons, and hard against the window's right edge with them.
    expect(documents.x + documents.width).toBeLessThanOrEqual(toolbar.x);
    expect(toolbar.x + toolbar.width).toBeLessThanOrEqual(settings.x);
    expect(toolbar.x).toBeGreaterThan(width / 2);
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
