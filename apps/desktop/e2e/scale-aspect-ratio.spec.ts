import { expect, test } from './helpers/app';

test('aspect ratio lock stays in sync and preserves proportions in both panels', async ({ session }, info) => {
  await session.openEditor();
  const { page } = session;
  await page.waitForFunction(() => !!(window as any).PM.Kernel.services.get('timeline')?.cv);
  await page.evaluate(() => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ name: 'Aspect ratio', dur: 5 });
    const layer = PM.mkLayer('solid', { name: 'Scale example', dur: 5 });
    layer.scaleLinked = false;
    layer.p['scale.x'].v = 180;
    layer.p['scale.y'].v = 90;
    project.layers.push(layer);
    PM.replaceProject(project);
    PM.selectLayers(layer.id);
    PM.Kernel.services.get('timeline').reveal(layer, ['scale.x', 'scale.y']);
    PM.bus.emit('layers');
    PM.invalidate();
  });
  const lock = page.getByRole('button', { name: 'Lock aspect ratio', exact: true });
  await expect(lock.locator('[data-icon="aspectRatio"][data-icon-set="phosphor"]')).toHaveCount(1);
  await expect(lock).toHaveAttribute('aria-pressed', 'false');
  await expect(lock).toHaveAttribute('title', 'Lock aspect ratio');
  await lock.click();
  await expect(lock).toHaveAttribute('aria-pressed', 'true');
  await expect(lock).toHaveAttribute('title', 'Unlock aspect ratio');

  const coordinates = () => page.evaluate(() => {
    const PM = (window as any).PM, timeline = PM.Kernel.services.get('timeline');
    const index = timeline.rows.findIndex((row: any) => row.key === 'scale');
    if (index < 0) throw new Error('Scale row is missing');
    const rect = timeline.cv.getBoundingClientRect();
    const width = (timeline.gut - 32 - timeline.propertyValueX - 8) / 2;
    return {
      x: rect.x + timeline.propertyValueX + width / 2,
      yAxis: rect.x + timeline.propertyValueX + width + 8 + width / 2,
      lock: rect.x + timeline.gut - 12,
      y: rect.y + timeline.ruler + index * timeline.row - timeline.scrollY + timeline.row / 2,
    };
  });
  const values = () => page.evaluate(() => {
    const PM = (window as any).PM, layer = PM.firstSel();
    return ['scale.x', 'scale.y'].map(key => PM.ev(layer, key, PM.time));
  });
  const typeScale = async (axis: 'X' | 'Y', value: string) => {
    const point = await coordinates();
    await page.mouse.click(axis === 'X' ? point.x : point.yAxis, point.y);
    const input = page.getByRole('textbox', { name: `Scale ${axis}`, exact: true });
    await input.fill(value);
    await input.press('Enter');
  };

  // Direct entry uses the clicked axis and preserves a non-square ratio.
  await typeScale('X', '240');
  expect(await values()).toEqual([240, 120]);
  await page.evaluate(() => (window as any).PM.hist.undo());
  expect(await values()).toEqual([180, 90]);
  await typeScale('Y', '120');
  expect(await values()).toEqual([240, 120]);
  await expect(page.getByRole('spinbutton', { name: 'Scale X', exact: true })).toHaveValue('240%');

  const point = await coordinates();
  await page.mouse.move(point.lock, point.y);
  await expect(page.locator('#tl-canvas-wrap canvas')).toHaveAttribute('title', 'Unlock aspect ratio');
  await page.mouse.click(point.lock, point.y);
  await expect(lock).toHaveAttribute('aria-pressed', 'false');
  await expect(page.locator('#tl-canvas')).toHaveAttribute('title', 'Lock aspect ratio');
  await typeScale('Y', '80');
  expect(await values()).toEqual([240, 80]);
  await page.screenshot({ path: info.outputPath('aspect-ratio-unlocked.png') });

  // Locking keeps the current proportions; scrubbing Y drives X too.
  await page.mouse.click(point.lock, point.y);
  await expect(lock).toHaveAttribute('aria-pressed', 'true');
  await page.mouse.move(point.yAxis, point.y);
  await page.mouse.down();
  await page.mouse.move(point.yAxis + 20, point.y, { steps: 4 });
  await page.mouse.up();
  const scrubbed = await values();
  expect(scrubbed[1]).toBeGreaterThan(80);
  expect(scrubbed[0]).toBeCloseTo(scrubbed[1] * 3);
  await page.evaluate(() => (window as any).PM.hist.undo());
  expect(await values()).toEqual([240, 80]);
  await page.mouse.move(point.yAxis, point.y);
  await page.mouse.down();
  await page.mouse.move(point.yAxis + 20, point.y, { steps: 4 });
  await page.keyboard.press('Escape');
  await page.mouse.up();
  expect(await values()).toEqual([240, 80]);

  const inspectorY = page.getByRole('spinbutton', { name: 'Scale Y', exact: true });
  await inspectorY.click();
  await inspectorY.fill('100');
  await inspectorY.press('Enter');
  expect(await values()).toEqual([300, 100]);
  await page.screenshot({ path: info.outputPath('aspect-ratio-locked.png') });

  // An animated axis keys its paired axis in the same undoable edit.
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.setKey(PM.firstSel(), 'scale.x', 0, 300);
    PM.setTime(1);
  });
  await typeScale('Y', '200');
  expect(await values()).toEqual([600, 200]);
  expect(await page.evaluate(() => {
    const PM = (window as any).PM, layer = PM.firstSel();
    return ['scale.x', 'scale.y'].map(key => layer.p[key].kf.some((frame: any) => frame.t === 1));
  })).toEqual([true, true]);
  await page.evaluate(() => (window as any).PM.hist.undo());
  expect(await values()).toEqual([300, 100]);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
