import { expect } from '@playwright/test';
import { test } from './helpers/app';
import type { Page } from '@playwright/test';

async function scaleFixture(page: Page) {
  await page.waitForFunction(() => Boolean((window as any).PM?.TL?.cv));
  return page.evaluate(() => {
    const PM = (window as any).PM;
    PM.replaceProject(PM.mkProject({ name: 'Timeline QA', dur: 10 }));
    const layer = PM.mkLayer('solid', { name: 'Scale QA', from: 2, dur: 3 });
    PM.proj.layers.push(layer);
    layer.scaleLinked = false;
    PM.setKey(layer, 'scale.x', 3, 100);
    PM.setKey(layer, 'scale.x', 4, 200);
    PM.setKey(layer, 'scale.y', 3, 50);
    PM.setKey(layer, 'scale.y', 4, 100);
    PM.selectLayers(layer.id);
    PM.sel.chan = 'scale';
    PM.TL.pps = 45;
    PM.TL.scrollT = 0;
    PM.TL.reveal(layer, ['scale.x', 'scale.y']);
    PM.setTime(3);
    PM.hist.clear();
    PM.invalidate();
    return layer.id;
  });
}

async function scaleKeyPoint(page: Page, time: number) {
  await expect.poll(() => page.evaluate(() => (window as any).PM.TL.rows.filter((row: any) => row.key === 'scale').length)).toBe(1);
  return page.evaluate((time) => {
    const T = (window as any).PM.TL;
    const box = T.cv.getBoundingClientRect();
    const index = T.rows.findIndex((row: any) => row.key === 'scale');
    return { x: box.x + T.gut + (time - T.scrollT) * T.pps,
      y: box.y + T.ruler + index * T.row - T.scrollY + T.row / 2, pps: T.pps };
  }, time);
}

test('Scale keys move outside the layer, delete together, and never delete their layer', async ({ session }) => {
  const { page } = session;
  const id = await scaleFixture(page);
  const point = await scaleKeyPoint(page, 3);
  const rows = await page.evaluate(() => (window as any).PM.TL.rows.map((row: any) => row.key).filter(Boolean));
  expect(rows).toEqual(['scale']);
  await page.mouse.move(point.x, point.y);
  await page.mouse.down();
  await page.mouse.move(point.x - 2 * point.pps, point.y, { steps: 8 });
  await page.mouse.up();
  expect(await page.evaluate((id) => {
    const PM = (window as any).PM, layer = PM.L(id);
    return { keys: PM.sel.keys.length, from: layer.from, dur: layer.dur,
      times: [layer.p['scale.x'].kf[0].t, layer.p['scale.y'].kf[0].t] };
  }, id)).toEqual({ keys: 2, from: 2, dur: 3, times: [-1, -1] });
  const later = await scaleKeyPoint(page, 4);
  await page.mouse.move(later.x, later.y);
  await page.mouse.down();
  await page.mouse.move(later.x + 2 * later.pps, later.y, { steps: 8 });
  await page.mouse.up();
  expect(await page.evaluate((id) => (window as any).PM.L(id).p['scale.x'].kf[1].t, id)).toBe(4);
  await page.keyboard.press('Backspace');
  await page.keyboard.press('Backspace');
  expect(await page.evaluate((id) => {
    const PM = (window as any).PM;
    return [PM.proj.layers.length, PM.L(id).p['scale.x'].kf.length, PM.L(id).p['scale.y'].kf.length];
  }, id)).toEqual([1, 1, 1]);
  await page.keyboard.press('Meta+z');
  expect(await page.evaluate((id) => (window as any).PM.L(id).p['scale.y'].kf.length, id)).toBe(2);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('the easing grid applies a curve to both Scale dimensions and preserves linear defaults', async ({ session }, testInfo) => {
  const { page } = session;
  const id = await scaleFixture(page);
  expect(await page.evaluate((id) => (window as any).PM.L(id).p['scale.x'].kf[0].eo, id)).toEqual([0, 0]);
  const point = await scaleKeyPoint(page, 3);
  await page.mouse.click(point.x, point.y, { button: 'right' });
  const grid = page.locator('[role="menu"].curve-grid');
  await expect(grid.locator('.curve-option')).toHaveCount(8);
  await expect(grid.locator('.curve-line')).toHaveCount(8);
  const boxes = await grid.locator('.curve-option').evaluateAll(elements => elements.map(el => {
    const b = el.getBoundingClientRect(); return { x: b.x, y: b.y };
  }));
  expect(boxes[0]!.y).toBe(boxes[3]!.y);
  expect(boxes[4]!.y).toBeGreaterThan(boxes[0]!.y);
  expect(boxes[4]!.x).toBe(boxes[0]!.x);
  await testInfo.attach('easing-grid', { body: await grid.screenshot(), contentType: 'image/png' });
  await grid.getByRole('menuitem', { name: 'Ease out', exact: true }).click();
  expect(await page.evaluate((id) => {
    const L = (window as any).PM.L(id);
    return [L.p['scale.x'].kf[0].ei, L.p['scale.y'].kf[0].ei];
  }, id)).toEqual([[.58, 1], [.58, 1]]);
  await page.keyboard.press('Meta+z');
  expect(await page.evaluate((id) => (window as any).PM.L(id).p['scale.y'].kf[0].ei, id)).toEqual([1, 1]);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

for (const descending of [false, true]) test(`incoming Bézier handles follow the pointer, undo and cancel (${descending ? 'falling' : 'rising'})`, async ({ session }) => {
  const { page } = session;
  const id = await scaleFixture(page);
  if (descending) await page.evaluate((id) => {
    const PM = (window as any).PM;
    PM.L(id).p['scale.x'].kf[1].v = 0;
    PM.L(id).p['scale.y'].kf[1].v = 0;
    PM.touch(); PM.invalidate();
  }, id);
  await page.getByRole('button', { name: 'Graph editor (G)', exact: true }).click();
  await page.waitForFunction((id) => {
    const PM = (window as any).PM;
    return PM.TL._graph?.series?.length === 2 && PM.UIState.getKeyHandles(PM.L(id).p['scale.x'].kf[1])?.hi;
  }, id);
  const point = await page.evaluate((id) => {
    const PM = (window as any).PM, T = PM.TL, box = T.cv.getBoundingClientRect();
    const pt = PM.UIState.getKeyHandles(PM.L(id).p['scale.x'].kf[1]).hi;
    return { x: box.x + pt[0], y: box.y + pt[1] };
  }, id);
  await page.mouse.move(point.x, point.y);
  await page.mouse.down();
  await page.mouse.move(point.x + 8, point.y - 12, { steps: 8 });
  await page.mouse.up();
  const result = await page.evaluate((id) => {
    const PM = (window as any).PM, L = PM.L(id);
    return [L.p['scale.x'].kf[1].ei, L.p['scale.y'].kf[1].ei];
  }, id);
  expect(result[0][0]).toBeGreaterThan(2 / 3);
  if (descending) expect(result[0][1]).toBeLessThan(2 / 3);
  else expect(result[0][1]).toBeGreaterThan(2 / 3);
  expect(result[1]).toEqual(result[0]);
  await page.keyboard.press('Meta+z');
  expect(await page.evaluate((id) => (window as any).PM.L(id).p['scale.x'].kf[1].ei, id)).toEqual([1, 1]);
  await page.mouse.move(point.x, point.y);
  await page.mouse.down();
  await page.mouse.move(point.x + 8, point.y - 12, { steps: 4 });
  await page.evaluate(() => window.dispatchEvent(new PointerEvent('pointercancel', { bubbles: true })));
  await page.mouse.up();
  expect(await page.evaluate((id) => (window as any).PM.L(id).p['scale.x'].kf[1].ei, id)).toEqual([1, 1]);
  expect(await page.evaluate((id) => (window as any).PM.L(id).p['scale.y'].kf[1].ei, id)).toEqual([1, 1]);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('project rename is available from the tab menu and persists the live document', async ({ session }) => {
  const { page } = session;
  const tab = page.locator('#tabs .project-doc.on');
  const id = await tab.getAttribute('data-tab-id');
  await tab.click({ button: 'right' });
  await page.getByRole('menuitem', { name: 'Rename project…' }).click();
  const input = page.getByRole('textbox', { name: 'Rename project', exact: true });
  await input.fill('Renamed composition');
  await input.press('Enter');
  await expect(tab).toContainText('Renamed composition');
  await page.evaluate(() => (window as any).PM.flushProject());
  expect(await page.evaluate((id) => (window as any).PM.Projects.get(id).name, id)).toBe('Renamed composition');
  await session.relaunch();
  await expect(session.page.locator('#tabs .project-doc.on')).toContainText('Renamed composition');
});

test('empty timeline clicks preserve the playhead and marquee selection still works', async ({ session }) => {
  const { page } = session;
  const id = await scaleFixture(page);
  const key = await scaleKeyPoint(page, 3);
  const points = await page.evaluate(() => {
    const T = (window as any).PM.TL, box = T.cv.getBoundingClientRect();
    const x = box.x + T.gut + T.pps; // 1s, before the 2s clip start.
    return {
      layer: { x, y: box.y + T.ruler + T.row / 2 },
      prop: { x, y: box.y + T.ruler + T.row * 1.5 },
      below: { x, y: box.y + T.ruler + T.rows.length * T.row + 12 }
    };
  });
  for (const point of Object.values(points)) {
    await page.mouse.click(point.x, point.y);
    expect(await page.evaluate(() => (window as any).PM.time)).toBe(3);
  }
  await page.mouse.move(key.x - 12, key.y - 10);
  await page.mouse.down();
  await page.mouse.move(key.x + 12, key.y + 10, { steps: 5 });
  await page.mouse.up();
  expect(await page.evaluate(() => (window as any).PM.sel.keys.length)).toBe(2);
  expect(await page.evaluate(() => (window as any).PM.time)).toBe(3);
  await page.getByRole('button', { name: 'Graph editor (G)', exact: true }).click();
  await page.waitForFunction(() => Boolean((window as any).PM.TL._graph));
  await page.mouse.click(points.prop.x, points.prop.y);
  expect(await page.evaluate(() => (window as any).PM.time)).toBe(3);
  expect(await page.evaluate((id) => (window as any).PM.L(id).from, id)).toBe(2);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('clicking the ruler moves the playhead', async ({ session }) => {
  const { page } = session;
  await page.waitForFunction(() => Boolean((window as any).PM?.TL?.cv));
  await page.waitForTimeout(800);
  const box = await page.locator('#tl-canvas').boundingBox();
  if (!box) throw new Error('no canvas');
  const gut = await page.evaluate(() => (window as any).PM.TL.gut);
  const ruler = await page.evaluate(() => (window as any).PM.TL.ruler);
  const before = await page.evaluate(() => (window as any).PM.time);
  // Click mid-ruler (below the thin work bar) well into the track area.
  await page.mouse.click(box.x + gut + 300, box.y + ruler - 6);
  const after = await page.evaluate(() => (window as any).PM.time);
  expect(after).not.toBe(before);
  expect(after).toBeGreaterThan(0);
  // Work area must be untouched by the scrub click.
  const work = await page.evaluate(() => (window as any).PM.proj.work);
  expect(work?.[0] ?? 0).toBe(0);
});

test('clicking the transport button pauses playback', async ({ session }) => {
  const { page } = session;
  const transport = page.getByRole('button', { name: 'Play / Pause (Space)' });

  await transport.click();
  await expect.poll(() => page.evaluate(() => (window as any).PM.playing)).toBe(true);
  await expect.poll(() => page.evaluate(() => (window as any).PM.time)).toBeGreaterThan(0);

  const box = await transport.boundingBox();
  if (!box) throw new Error('no transport button');
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.waitForTimeout(100);
  await page.mouse.up();
  await expect.poll(() => page.evaluate(() => (window as any).PM.playing)).toBe(false);
  const pausedAt = await page.evaluate(() => (window as any).PM.time);
  await page.waitForTimeout(150);
  expect(await page.evaluate(() => (window as any).PM.time)).toBe(pausedAt);
});
