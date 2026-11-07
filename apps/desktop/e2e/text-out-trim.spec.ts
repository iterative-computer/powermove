import { test, expect } from './helpers/app';

const proof = process.env.PM_OUT_TRIM_PROOF;
const directory = '/tmp/pm-figuate-feedback/shots/text-out-anim-trim';

for (const mode of ['layers', 'tracks'] as const) test(`an exit ramp follows a live right trim and exits at the new out point, with one Undo (${mode})`, async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  const capture = proof && mode === 'layers';
  await page.waitForFunction(() => (window as any).PM.GL?.gl);
  await page.evaluate((baseline) => {
    const PM = (window as any).PM, timeline = PM.Kernel.services.get('timeline');
    PM.replaceProject(PM.mkProject({ name: 'Exit trim', w: 640, h: 360, fps: 30, dur: 6, bg: '#000000' }));
    const layer = PM.mkLayer('text', { name: 'Exit follows right edge', from: 0, dur: 4,
      p: { 'position.x': 320, 'position.y': 180 }, d: { text: 'EXIT', size: 80, color: '#ffffff', align: 'center' } });
    layer.d.animators = [{ id: 'exit', name: 'Exit', mode: 'stagger', direction: 'out', easing: 'linear', unit: 'characters',
      timingVersion: 2, p: Object.fromEntries(Object.entries({ delay: baseline ? 2.7 : 0, duration: 1, stagger: .1, opacity: 0 })
        .map(([key, value]) => [key, PM.P(value)])) }];
    PM.proj.layers = [layer]; PM.ProjectIndex.invalidate(); PM.touch(); PM.selectLayers(layer.id); PM.hist.clear();
    timeline.pps = 100; timeline.scrollT = 0; timeline.graph = false;
    PM.setTime(2.9, { force: true }); PM.bus.emit('layers'); PM.invalidate();
    // Record the actual painted cascade polygons, including the live drag.
    const ctx = timeline.cv.getContext('2d'), points: number[][] = [];
    const begin = ctx.beginPath.bind(ctx), move = ctx.moveTo.bind(ctx), line = ctx.lineTo.bind(ctx), fill = ctx.fill.bind(ctx);
    (window as any).exitRamps = [];
    ctx.beginPath = () => { points.length = 0; begin(); };
    ctx.moveTo = (x: number, y: number) => { points.push([x, y]); move(x, y); };
    ctx.lineTo = (x: number, y: number) => { points.push([x, y]); line(x, y); };
    ctx.fill = (...args: any[]) => {
      if (points.length === 4 && points[0][1] === points[1][1] && points[2][1] === points[3][1]
        && points[2][1] - points[0][1] >= 4 && points[2][1] - points[0][1] <= 8)
        (window as any).exitRamps.push(Math.max(...points.map(p => p[0])));
      fill(...args);
    };
    PM.invalidate();
  }, proof === 'before');
  await page.locator(`#tl-mode [data-mode="${mode}"]`).click();
  const coordinates = await page.evaluate(mode => {
    const timeline = (window as any).PM.Kernel.services.get('timeline'), r = timeline.cv.getBoundingClientRect();
    const index = timeline.rows.findIndex((row: any) => row.kind === 'layer');
    const y = mode === 'tracks' ? timeline.trackView.laneY('video', 0) + 8
      : timeline.ruler + index * timeline.row - timeline.scrollY + 8;
    return { x: r.x + timeline.gut + 400, y: r.y + y,
      newRampEnd: timeline.gut + 300 };
  }, mode);
  // Positive coverage before the exit prevents an empty renderer from passing.
  expect(await page.evaluate(() => {
    const PM = (window as any).PM;
    return [...PM.GL.renderToPixels(1, 320, 180, { transparent: true, mblur: false })]
      .filter((_, i) => i % 4 === 3).reduce((sum, v) => sum + Number(v), 0);
  })).toBeGreaterThan(10000);
  if (capture) await page.screenshot({ path: `${directory}/${proof === 'before' ? '01-before' : '02-after'}-timeline-before-trim.png` });
  await page.mouse.move(coordinates.x - 1, coordinates.y); await page.mouse.down();
  await page.mouse.move(coordinates.x - 101, coordinates.y, { steps: 8 });
  await expect.poll(() => page.evaluate(() => (window as any).PM.proj.layers[0].dur)).toBe(3);
  if (proof !== 'before') {
    await expect.poll(() => page.evaluate(() => (window as any).exitRamps.at(-1))).toBeCloseTo(coordinates.newRampEnd, 0);
  }
  await page.mouse.up();
  if (capture) await page.screenshot({ path: `${directory}/${proof === 'before' ? '01-before' : '02-after'}-timeline-after-trim.png` });
  const alpha = await page.evaluate(() => {
    const PM = (window as any).PM;
    return [...PM.GL.renderToPixels(2.9999, 320, 180, { transparent: true, mblur: false })]
      .filter((_, i) => i % 4 === 3).reduce((sum, v) => sum + Number(v), 0);
  });
  if (proof !== 'before') expect(alpha).toBeLessThan(100);
  else expect(alpha).toBeGreaterThan(10000);
  if (capture) {
    for (const time of [2.5, 2.9]) {
      await page.evaluate(time => (window as any).PM.setTime(time, { force: true }), time);
      await page.screenshot({ path: `${directory}/${proof === 'before' ? '01-before' : '02-after'}-viewer-${time}.png` });
    }
  }
  await page.evaluate(() => (window as any).PM.hist.undo());
  expect(await page.evaluate(() => (window as any).PM.proj.layers[0].dur)).toBe(4);
  await page.evaluate(() => (window as any).PM.hist.redo());
  expect(await page.evaluate(() => (window as any).PM.proj.layers[0].dur)).toBe(3);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
