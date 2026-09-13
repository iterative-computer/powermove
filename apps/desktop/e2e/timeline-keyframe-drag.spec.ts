import { expect } from '@playwright/test';
import { test } from './helpers/app';

for (const path of ['c.color', 'position.y']) test(`drag ${path} keyframes without changing values, then Undo`, async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await page.waitForFunction(() => { const PM = (window as any).PM; const timeline = PM.Kernel.services.get('timeline'); return !!timeline?.cv; });
  const before = await page.evaluate(path => {
    const PM = (window as any).PM;
    PM.replaceProject(PM.mkProject({ name: 'Keyframe drag', dur: 6 }));
    const L = PM.mkLayer('text', { dur: 6 });
    PM.proj.layers.push(L); PM.selectLayers(L.id);
    for (const [time, value] of path === 'c.color' ? [[1, '#ff0000'], [3, '#0000ff']] : [[1, 100], [3, 300]]) {
      PM.Edit.apply({ type: 'set_property', target: L.id, path, time, value, mode: 'keyframe', preserveHandEdits: false });
    }
    PM.UIState.setLayerCollapsed(L, false); PM.bus.emit('layers'); PM.setTime(0);
    return JSON.stringify(PM.findProp(L, path).kf);
  }, path);
  await expect.poll(() => page.evaluate(path => { const PM = (window as any).PM; const timeline = PM.Kernel.services.get('timeline'); return timeline.rows.some((r: any) => r.key === path); }, path)).toBe(true);
  const point = await page.evaluate(path => {
    const PM = (window as any).PM;
    const timeline = PM.Kernel.services.get('timeline');
    const rect = timeline.cv.getBoundingClientRect();
    const i = timeline.rows.findIndex((r: any) => r.key === path);
    return { x: rect.x + timeline.gut + (1 - timeline.scrollT) * timeline.pps, y: rect.y + timeline.ruler + i * timeline.row - timeline.scrollY + timeline.row / 2, dx: timeline.pps };
  }, path);
  await page.mouse.move(point.x, point.y); await page.mouse.down();
  await page.mouse.move(point.x + point.dx, point.y, { steps: 8 }); await page.mouse.up();
  const keys = () => page.evaluate(path => (window as any).PM.findProp((window as any).PM.proj.layers[0], path).kf, path);
  await expect.poll(async () => (await keys())[0].t).toBe(2);
  const moved = await keys(), original = JSON.parse(before);
  expect(moved.map((k: any) => ({ ...k, t: undefined }))).toEqual(original.map((k: any) => ({ ...k, t: undefined })));
  expect(await page.evaluate(() => (window as any).PM.time)).toBe(0);
  await page.evaluate(() => (window as any).PM.hist.undo());
  expect(JSON.stringify(await keys())).toBe(before);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
