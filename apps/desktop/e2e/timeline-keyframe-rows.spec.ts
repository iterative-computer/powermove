import { expect } from '@playwright/test';
import { test } from './helpers/app';

test('expanding a layer strip immediately shows its existing keyframed properties', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await page.waitForFunction(() => { const PM = (window as any).PM; const timeline = PM.Kernel.services.get('timeline'); return Boolean(timeline?.cv); });
  const layerId = await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.replaceProject(PM.mkProject({ name: 'Timeline keyframe rows', dur: 5 }));
    const layer = PM.mkLayer('solid', { name: 'Animated layer', dur: 5 });
    PM.proj.layers.push(layer);
    PM.setKey(layer, 'opacity', 0, 25);
    PM.setKey(layer, 'opacity', 2, 100);
    PM.UIState.setLayerCollapsed(layer, true);
    PM.bus.emit('layers');
    PM.invalidate('timeline');
    return layer.id;
  });

  await expect.poll(() => page.evaluate(() => { const PM = (window as any).PM; const timeline = PM.Kernel.services.get('timeline'); return timeline.rows.length; })).toBe(1);
  const disclosure = await page.evaluate((id) => {
    const PM = (window as any).PM;
    const timeline = PM.Kernel.services.get('timeline');

    const row = timeline.rows.findIndex((item: any) => item.kind === 'layer' && item.L.id === id);
    const box = timeline.cv.getBoundingClientRect();
    return {
      x: box.x + 66,
      y: box.y + timeline.ruler + row * timeline.row - timeline.scrollY + timeline.row / 2,
    };
  }, layerId);
  await page.mouse.click(disclosure.x, disclosure.y);

  await expect.poll(() => page.evaluate((id) => {
    const PM = (window as any).PM;
    const timeline = PM.Kernel.services.get('timeline');
    return timeline.rows
      .filter((row: any) => row.kind === 'prop' && row.L.id === id)
      .map((row: any) => row.key);
  }, layerId)).toContain('opacity');
  expect(session.diagnostics.pageErrors).toEqual([]);
});


test('disclosure shows only keyframed properties and refreshes animation changes', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await page.waitForFunction(() => { const PM = (window as any).PM; const timeline = PM.Kernel.services.get('timeline'); return Boolean(timeline?.cv); });
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.replaceProject(PM.mkProject({ name: 'Common timeline controls', dur: 5 }));
    const layer = PM.mkLayer('solid', { name: 'Unanimated layer', dur: 5 });
    PM.proj.layers.push(layer);
    PM.UIState.setLayerCollapsed(layer, true);
    PM.bus.emit('layers');
    PM.invalidate('timeline');
  });
  await expect.poll(() => page.evaluate(() => { const PM = (window as any).PM; const timeline = PM.Kernel.services.get('timeline'); return timeline.rows.length; })).toBe(1);
  const point = await page.evaluate(() => {
    const PM = (window as any).PM;
    const timeline = PM.Kernel.services.get('timeline');
    const box = timeline.cv.getBoundingClientRect();
    return { x: box.x + 66, y: box.y + timeline.ruler + timeline.row / 2 };
  });
  await page.mouse.click(point.x, point.y);
  const rows = () => page.evaluate(() => { const PM = (window as any).PM; const timeline = PM.Kernel.services.get('timeline'); return timeline.rows.filter((r: any) => r.kind === 'prop').map((r: any) => r.key); });
  await expect.poll(rows).toEqual([]);
  expect(await rows()).not.toContain('skew');
  // The same stopwatch operation used by Properties, without selection changes.
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.toggleStopwatch(PM.proj.layers[0], 'skew', 0);
  });
  await expect.poll(rows).toEqual(['skew']);
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.toggleStopwatch(PM.proj.layers[0], 'skew', 0);
  });
  await expect.poll(rows).not.toContain('skew');
  await page.mouse.click(point.x, point.y);
  await expect.poll(() => page.evaluate(() => { const PM = (window as any).PM; const timeline = PM.Kernel.services.get('timeline'); return timeline.rows.length; })).toBe(1);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
