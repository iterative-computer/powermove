import { expect, test } from './helpers/app';
import type { Page } from 'playwright';

async function fieldPoint(page: Page, key: string, axis = 0) {
  return page.evaluate(({ key, axis }) => {
    const timeline = (window as any).PM.Kernel.services.get('timeline');
    const view = timeline.trackView, rect = timeline.cv.getBoundingClientRect();
    for (let index = 0; index < 2; index++) {
      const y = view.laneY('video', 0) + view.rowHeight() + index * 24 + 12;
      if (view.rowAt(timeline.gut + 10, y)?.key === key) {
        return { x: rect.left + timeline.gut - (key === 'scale' ? (axis === 0 ? 100 : 52) : 44), y: rect.top + y };
      }
    }
    throw new Error(`Missing track property ${key}`);
  }, { key, axis });
}

async function scaleLockPoint(page: Page) {
  const point = await fieldPoint(page, 'scale');
  return { x: point.x + 88, y: point.y };
}

test('track property values support typing, axis-specific dragging, cancellation, Undo, and locks', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  const id = await page.evaluate(() => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ w: 640, h: 360, dur: 5 });
    const layer = PM.mkLayer('solid', { name: 'Track value QA', from: 0, dur: 5 }, project);
    layer.scaleLinked = false;
    layer.p['scale.y'].v = 50;
    project.layers = [layer];
    PM.replaceProject(project); PM.selectLayers(layer.id); PM.setTime(0);
    PM.Kernel.services.get('timeline').reveal(layer, ['scale.x', 'scale.y', 'position.x']);
    PM.hist.clear(); PM.invalidate();
    return layer.id;
  });
  await page.locator('#tl-mode [data-mode="tracks"]').click();
  const scales = () => page.evaluate(id => {
    const layer = (window as any).PM.L(id);
    return [layer.p['scale.x'].v, layer.p['scale.y'].v];
  }, id);
  const x = await fieldPoint(page, 'scale');
  await page.mouse.click(x.x, x.y);
  const xInput = page.getByRole('textbox', { name: 'Scale X', exact: true });
  await expect(xInput).toBeVisible();
  const bounds = (await xInput.boundingBox())!;
  expect(x.x).toBeGreaterThanOrEqual(bounds.x);
  expect(x.x).toBeLessThanOrEqual(bounds.x + bounds.width);
  expect(x.y).toBeGreaterThanOrEqual(bounds.y);
  expect(x.y).toBeLessThanOrEqual(bounds.y + bounds.height);
  await xInput.fill('120'); await xInput.press('Enter');
  await expect.poll(scales).toEqual([120, 50]);
  await page.evaluate(() => (window as any).PM.hist.undo());
  await expect.poll(scales).toEqual([100, 50]);

  const y = await fieldPoint(page, 'scale', 1);
  await page.mouse.click(y.x, y.y);
  const yInput = page.getByRole('textbox', { name: 'Scale Y', exact: true });
  await yInput.fill('75'); await yInput.press('Escape');
  await expect.poll(scales).toEqual([100, 50]);
  await page.mouse.click(y.x, y.y);
  await yInput.fill('75'); await yInput.press('Enter');
  await expect.poll(scales).toEqual([100, 75]);

  await page.mouse.move(y.x, y.y); await page.mouse.down();
  await page.mouse.move(y.x + 20, y.y, { steps: 5 }); await page.mouse.up();
  await expect.poll(scales).toEqual([100, 85]);
  await page.evaluate(() => (window as any).PM.hist.undo());
  await expect.poll(scales).toEqual([100, 75]);

  const lockPoint = await scaleLockPoint(page);
  const inspectorLock = page.getByRole('button', { name: 'Lock aspect ratio', exact: true });
  const canvas = page.locator('#tl-canvas');
  await page.mouse.move(lockPoint.x, lockPoint.y);
  await expect(canvas).toHaveAttribute('title', 'Lock aspect ratio');
  await expect(inspectorLock).toHaveAttribute('aria-pressed', 'false');
  await page.mouse.click(lockPoint.x, lockPoint.y);
  await expect(inspectorLock).toHaveAttribute('aria-pressed', 'true');
  await expect(page.locator('#tl-canvas-wrap > input')).toHaveCount(0);
  await page.evaluate(() => (window as any).PM.hist.undo());
  await expect(inspectorLock).toHaveAttribute('aria-pressed', 'false');
  await page.mouse.click(lockPoint.x, lockPoint.y);
  await expect(inspectorLock).toHaveAttribute('aria-pressed', 'true');
  await page.mouse.click(x.x, x.y);
  await xInput.fill('200'); await xInput.press('Enter');
  await expect.poll(scales).toEqual([200, 150]);
  await page.mouse.move(x.x, x.y); await page.mouse.down();
  await page.mouse.move(x.x + 20, x.y, { steps: 5 });
  await expect.poll(scales).toEqual([210, 157.5]);
  await page.keyboard.press('Escape'); await page.mouse.up();
  await expect.poll(scales).toEqual([200, 150]);

  for (const scheme of ['light', 'dark']) {
    await page.evaluate(scheme => { const PM = (window as any).PM; PM.theme.apply(scheme); PM.invalidate(); }, scheme);
    const themePoint = await fieldPoint(page, 'scale', 1);
    const themeLockPoint = await scaleLockPoint(page);
    await page.mouse.move(themeLockPoint.x, themeLockPoint.y);
    await expect(canvas).toHaveAttribute('title', 'Unlock aspect ratio');
    await page.mouse.dblclick(themePoint.x, themePoint.y);
    await expect(yInput).toBeVisible();
    await expect(page.locator('#tl-canvas-wrap > input')).toHaveCount(1);
    const clip = await page.evaluate(() => {
      const timeline = (window as any).PM.Kernel.services.get('timeline');
      const rect = timeline.cv.getBoundingClientRect();
      return { x: rect.left, y: rect.top, width: timeline.gut, height: timeline.trackView.laneY('audio', 0) };
    });
    await page.screenshot({ path: `/tmp/powermove-track-values-${scheme}.png`, clip });
    await yInput.press('Escape');
  }

  await inspectorLock.click();
  const currentLockPoint = await scaleLockPoint(page);
  await page.mouse.move(currentLockPoint.x, currentLockPoint.y);
  await expect(canvas).toHaveAttribute('title', 'Lock aspect ratio');
  await page.mouse.click(currentLockPoint.x, currentLockPoint.y);
  await expect(inspectorLock).toHaveAttribute('aria-pressed', 'true');

  const position = await fieldPoint(page, 'position.x');
  await page.mouse.click(position.x, position.y);
  const positionInput = page.locator('#tl-canvas-wrap > input');
  await expect(positionInput).toBeVisible();
  await positionInput.fill('320'); await positionInput.press('Enter');
  expect(await page.evaluate(id => (window as any).PM.L(id).p['position.x'].v, id)).toBe(320);

  await page.evaluate(id => { const PM = (window as any).PM; PM.L(id).lock = true; PM.invalidate(); }, id);
  const lockedX = await fieldPoint(page, 'scale');
  const lockedY = await fieldPoint(page, 'scale', 1);
  await page.mouse.click(lockedX.x, lockedX.y);
  await expect(xInput).toHaveCount(0);
  await page.mouse.move(lockedY.x, lockedY.y); await page.mouse.down();
  await page.mouse.move(lockedY.x + 20, lockedY.y, { steps: 5 }); await page.mouse.up();
  await expect.poll(scales).toEqual([200, 150]);
  const lockedLockPoint = await scaleLockPoint(page);
  await page.mouse.click(lockedLockPoint.x, lockedLockPoint.y);
  expect(await page.evaluate(id => (window as any).PM.L(id).scaleLinked, id)).toBe(true);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('a track readout edits the clip at the playhead and creates paired Scale keys', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  const ids = await page.evaluate(() => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ w: 640, h: 360, dur: 5 });
    const first = PM.mkLayer('solid', { name: 'First clip', from: 0, dur: 1 }, project);
    const active = PM.mkLayer('solid', { name: 'Active clip', from: 3, dur: 1 }, project);
    active.scaleLinked = true;
    active.p['scale.y'].v = 50;
    project.layers = [first, active];
    PM.replaceProject(project);
    PM.setKey(active, 'scale.x', 3, 100);
    PM.selectLayers(first.id); PM.setTime(3.5);
    const timeline = PM.Kernel.services.get('timeline');
    for (const layer of project.layers) timeline.reveal(layer, ['scale.x', 'scale.y']);
    PM.hist.clear(); PM.invalidate();
    return { first: first.id, active: active.id };
  });
  await page.locator('#tl-mode [data-mode="tracks"]').click();
  const point = await fieldPoint(page, 'scale');
  await page.mouse.click(point.x, point.y);
  const input = page.getByRole('textbox', { name: 'Scale X', exact: true });
  await expect(input).toHaveValue('100');
  await input.fill('160'); await input.press('Enter');
  expect(await page.evaluate(ids => {
    const PM = (window as any).PM, active = PM.L(ids.active);
    return { selected: PM.sel.layers, first: PM.L(ids.first).p['scale.x'].v,
      values: ['scale.x', 'scale.y'].map(key => PM.ev(active, key, PM.time)),
      paired: ['scale.x', 'scale.y'].map(key => active.p[key].kf.some((frame: any) => frame.t === .5)) };
  }, ids)).toEqual({ selected: [ids.active], first: 100, values: [160, 80], paired: [true, true] });
  await page.evaluate(() => (window as any).PM.hist.undo());
  expect(await page.evaluate(id => {
    const PM = (window as any).PM, layer = PM.L(id);
    return ['scale.x', 'scale.y'].map(key => PM.ev(layer, key, PM.time));
  }, ids.active)).toEqual([100, 50]);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
