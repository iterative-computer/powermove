import { expect } from '@playwright/test';
import { test } from './helpers/app';

test.beforeEach(async ({ session }) => { await session.openEditor(); });

test('AE property toggles and global M disclosure use actual timeline rows', async ({ session }) => {
  const { page } = session;
  await page.waitForFunction(() => { const PM = (window as any).PM, timeline = PM.Kernel.services.get('timeline'); return Boolean(timeline?.cv); });
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.replaceProject(PM.mkProject({ name: 'Property shortcuts' }));
    const L = PM.mkLayer('solid');
    PM.proj.layers.push(L, PM.mkLayer('solid'));
    PM.selectLayers([L.id]);
    PM.UIState.setLayerCollapsed(L, true);
    PM.bus.emit('layers');
    (document.activeElement as HTMLElement)?.blur();
  });
  const rows = () => page.evaluate(() => { const PM = (window as any).PM, timeline = PM.Kernel.services.get('timeline'); return timeline.rows.filter((r: any) => r.kind === 'prop').map((r: any) => r.key); });
  await page.keyboard.press('p');
  await expect.poll(rows).toEqual(['position.x', 'position.y']);
  await page.keyboard.press('Shift+t');
  await expect.poll(rows).toEqual(['position.x', 'position.y', 'opacity']);
  await page.keyboard.press('Shift+t');
  // TT reveals mask opacity, restoring the pre-first-tap set for Shift combinations.
  await page.keyboard.press('s');
  await expect.poll(rows).toEqual(['scale']);
  await page.keyboard.press('p');
  await page.keyboard.press('t');
  await expect.poll(rows).toEqual(['opacity']);
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.selectLayers([]);
    PM.proj.layers.forEach((L: any) => PM.UIState.setLayerCollapsed(L, true));
    PM.bus.emit('layers');
  });
  await page.keyboard.press('m');
  await expect.poll(async () => (await rows()).length).toBeGreaterThan(4);
  await page.keyboard.press('m');
  await expect.poll(rows).toEqual([]);
  await page.keyboard.press('m');
  await expect.poll(async () => (await rows()).length).toBeGreaterThan(4);
  const count = (await rows()).length;
  await page.evaluate(() => { const input = document.createElement('input'); input.id = 'shortcut-field'; document.body.append(input); input.focus(); });
  await page.keyboard.type('mpsratufel');
  await expect(page.locator('#shortcut-field')).toHaveValue('mpsratufel');
  expect((await rows()).length).toBe(count);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('timeline diamond toggles playhead keys without clearing other times', async ({ session }) => {
  const { page } = session;
  await page.waitForFunction(() => { const PM = (window as any).PM, timeline = PM.Kernel.services.get('timeline'); return Boolean(timeline?.cv); });
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.replaceProject(PM.mkProject({ name: 'Key toggle' }));
    const L = PM.mkLayer('solid'); PM.proj.layers.push(L); PM.selectLayers([L.id]);
    PM.setKey(L, 'opacity', 0, 20); PM.setKey(L, 'opacity', 2, 80);
    const timeline = PM.Kernel.services.get('timeline');
    PM.time = 1; timeline.reveal(L, ['opacity']); PM.bus.emit('layers'); PM.invalidate();
  });
  const keys = () => page.evaluate(() => (window as any).PM.proj.layers[0].p.opacity.kf.map((k: any) => k.t));
  await expect.poll(async () => page.evaluate(() => { const PM = (window as any).PM, timeline = PM.Kernel.services.get('timeline'); return timeline.rows.some((r: any) => r.key === 'opacity'); })).toBe(true);
  const point = await page.evaluate(() => {
    const PM = (window as any).PM, timeline = PM.Kernel.services.get('timeline'); const box = timeline.cv.getBoundingClientRect();
    return { x: box.x + 85, y: box.y + timeline.ruler + timeline.rows.findIndex((r: any) => r.key === 'opacity') * timeline.row - timeline.scrollY + timeline.row / 2 };
  });
  await page.mouse.click(point.x, point.y);
  await expect.poll(keys).toEqual([0, 1, 2]);
  await page.mouse.click(point.x, point.y);
  await expect.poll(keys).toEqual([0, 2]);
  await page.evaluate(() => (window as any).PM.hist.undo());
  await expect.poll(keys).toEqual([0, 1, 2]);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
