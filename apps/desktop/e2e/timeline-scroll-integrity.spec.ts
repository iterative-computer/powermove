import { expect, test } from './helpers/app';

test('large timeline stays usable after vertical scrolling, panning, zooming and scrubbing', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await page.evaluate(() => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ dur: 30 });
    project.layers = Array.from({ length: 258 }, (_, index) => PM.mkLayer('solid', { name: `Layer ${index}`, from: 0, dur: 30 }, project));
    PM.replaceProject(project);
    const timeline = PM.Kernel.services.get('timeline');
    timeline.setMode('layers'); timeline.pps = 90; timeline.scrollT = timeline.scrollY = 0;
    PM.setTime(2); PM.invalidate();
  });
  await page.waitForFunction(() => (window as any).PM.Kernel.services.get('timeline').rows.length >= 258);
  const point = await page.evaluate(() => {
    const timeline = (window as any).PM.Kernel.services.get('timeline');
    const box = timeline.cv.getBoundingClientRect();
    return { x: box.x + timeline.gut + 100, y: box.y + timeline.ruler + 80, rulerY: box.y + 18 };
  });
  await page.mouse.move(point.x, point.y);
  await page.mouse.wheel(0, 5000);
  await expect.poll(() => page.evaluate(() => (window as any).PM.Kernel.services.get('timeline').scrollY)).toBeGreaterThan(1000);
  await page.mouse.wheel(180, 0);
  await expect.poll(() => page.evaluate(() => (window as any).PM.Kernel.services.get('timeline').scrollT)).toBeCloseTo(2);
  await page.keyboard.down('Control'); await page.mouse.wheel(0, -80); await page.keyboard.up('Control');
  await expect.poll(() => page.evaluate(() => (window as any).PM.Kernel.services.get('timeline').pps)).toBeGreaterThan(90);
  await page.mouse.click(point.x, point.rulerY);
  const state = await page.evaluate(() => {
    const PM = (window as any).PM, timeline = PM.Kernel.services.get('timeline');
    const time = PM.time;
    PM.setTime(NaN); PM.setTime(Infinity);
    return { finite: [PM.time, timeline.scrollT, timeline.scrollY, timeline.pps].every(Number.isFinite), time, after: PM.time, readout: document.querySelector('#tl-time')?.textContent };
  });
  expect(state.finite).toBe(true);
  expect(state.after).toBe(state.time);
  expect(state.readout).not.toContain('NaN');
  const zoom = await page.evaluate(() => {
    const timeline = (window as any).PM.Kernel.services.get('timeline');
    const box = timeline.cv.getBoundingClientRect();
    const wheel = (deltaY: number) => timeline.cv.dispatchEvent(new WheelEvent('wheel', {
      ctrlKey: true, deltaY, clientX: box.x + timeline.gut + 100, clientY: box.y + 80, cancelable: true,
    }));
    for (let index = 0; index < 50; index++) wheel(-1000);
    const max = timeline.pps;
    for (let index = 0; index < 50; index++) wheel(1000);
    const min = timeline.pps;
    timeline.pps = 0; timeline.scrollT = NaN;
    wheel(-80);
    return { max, min, recovered: Number.isFinite(timeline.scrollT) && Number.isFinite(timeline.pps) && timeline.pps > 0 };
  });
  expect(zoom).toEqual({ max: 4000, min: 4, recovered: true });
  expect(session.diagnostics.pageErrors).toEqual([]);
});
