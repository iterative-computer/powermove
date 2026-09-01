import { expect } from '@playwright/test';
import { test } from './helpers/app';

test('expanding a layer strip immediately shows its existing keyframed properties', async ({ session }) => {
  const { page } = session;
  await page.waitForFunction(() => Boolean((window as any).PM?.TL?.cv));
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

  await expect.poll(() => page.evaluate(() => (window as any).PM.TL.rows.length)).toBe(1);
  const disclosure = await page.evaluate((id) => {
    const PM = (window as any).PM;
    const T = PM.TL;
    const row = T.rows.findIndex((item: any) => item.kind === 'layer' && item.L.id === id);
    const box = T.cv.getBoundingClientRect();
    return {
      x: box.x + 66,
      y: box.y + T.ruler + row * T.row - T.scrollY + T.row / 2,
    };
  }, layerId);
  await page.mouse.click(disclosure.x, disclosure.y);

  await expect.poll(() => page.evaluate((id) => {
    const PM = (window as any).PM;
    return PM.TL.rows
      .filter((row: any) => row.kind === 'prop' && row.L.id === id)
      .map((row: any) => row.key);
  }, layerId)).toContain('opacity');
  expect(session.diagnostics.pageErrors).toEqual([]);
});
