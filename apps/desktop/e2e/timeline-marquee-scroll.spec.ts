import { expect, test } from './helpers/app';

for (const mode of ['layers', 'tracks']) {
  test(`${mode} selection origin follows the timeline while scrolling with the pointer held still`, async ({ session }) => {
    await session.openEditor();
    const { page } = session;
    await page.evaluate((mode) => {
      const PM = (window as any).PM;
      const project = PM.mkProject({ dur: 30 });
      project.layers = Array.from({ length: 60 }, (_, index) => PM.mkLayer('solid', { name: `Layer ${index}`, from: 0, dur: 1 }, project));
      PM.replaceProject(project);
      const timeline = PM.Kernel.services.get('timeline');
      timeline.setMode(mode); timeline.pps = 90;
      timeline.scrollT = 2; timeline.scrollY = timeline.trackScrollY = 100;
      PM.invalidate();
    }, mode);
    await page.waitForFunction(() => (window as any).PM.Kernel.services.get('timeline').rows.length >= 60);
    const origin = await page.evaluate(() => {
      const t = (window as any).PM.Kernel.services.get('timeline');
      const box = t.cv.getBoundingClientRect();
      return { x: box.x + t.gut + 150, y: box.y + t.ruler + 30 };
    });
    await page.mouse.move(origin.x, origin.y);
    await page.mouse.down();
    await page.mouse.move(origin.x + 100, origin.y + 80);
    const read = () => page.evaluate(() => {
      const t = (window as any).PM.Kernel.services.get('timeline');
      return { ...t.marquee, scrollT: t.scrollT, scrollY: t.mode === 'tracks' ? t.trackScrollY : t.scrollY, pps: t.pps };
    });
    await expect.poll(async () => (await read()).x0).toBeDefined();
    const before = await read();
    // No mousemove follows the wheel: scrolling itself must update the box.
    for (const [dx, dy] of [[45, 20], [-45, -20], [0, 20], [0, -20]]) {
      const previous = await read();
      await page.mouse.wheel(dx, dy);
      await expect.poll(async () => {
        const now = await read();
        return dx ? now.scrollT !== previous.scrollT : now.scrollY !== previous.scrollY;
      }).toBe(true);
      await expect.poll(async () => {
        const now = await read();
        return [Math.round(now.x0 + (now.scrollT - before.scrollT) * now.pps), Math.round(now.y0 + now.scrollY - before.scrollY)];
      }).toEqual([Math.round(before.x0), Math.round(before.y0)]);
      const now = await read();
      expect(now.x1).toBeCloseTo(before.x1);
      expect(now.y1).toBeCloseTo(before.y1);
    }
    await page.mouse.move(origin.x + 110, origin.y + 90);
    const moved = await read();
    expect(moved.x0).toBeCloseTo(before.x0);
    expect(moved.y0).toBeCloseTo(before.y0);
    await page.mouse.up();
    expect(await page.evaluate(() => (window as any).PM.Kernel.services.get('timeline').marquee)).toBeNull();
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
}

test('graph selection origin stays at the same time and value while panning', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await page.evaluate(() => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ dur: 10 });
    const layer = PM.mkLayer('solid', { name: 'Graph scroll', from: 0, dur: 10 }, project);
    project.layers = [layer]; PM.replaceProject(project);
    PM.setKey(layer, 'position.x', 3, 100);
    PM.setKey(layer, 'position.x', 5, 200);
    PM.selectLayers(layer.id); PM.sel.chan = 'position.x';
    PM.sel.keys = layer.p['position.x'].kf.map((key: any) => key.i);
    const t = PM.Kernel.services.get('timeline');
    t.setMode('layers'); t.pps = 45; t.scrollT = 0;
    t.keySelectionActive = true;
    t.reveal(layer, ['position.x']);
    PM.bus.emit('sel'); PM.invalidate();
  });
  await page.getByRole('button', { name: 'Graph editor (Shift+F3)', exact: true }).click();
  await page.waitForFunction(() => (window as any).PM.Kernel.services.get('timeline')._graph?.points.length === 2);
  const corners = await page.evaluate(() => {
    const t = (window as any).PM.Kernel.services.get('timeline');
    const box = t.cv.getBoundingClientRect(), points = t._graph.points;
    return { x0: box.x + Math.min(...points.map((p: any) => p.x)) - 20,
      y0: box.y + Math.min(...points.map((p: any) => p.y)) - 10,
      x1: box.x + Math.max(...points.map((p: any) => p.x)) + 20,
      y1: box.y + Math.max(...points.map((p: any) => p.y)) + 10 };
  });
  await page.mouse.move(corners.x0, corners.y0); await page.mouse.down();
  await page.mouse.move(corners.x1, corners.y1);
  const read = () => page.evaluate(() => {
    const t = (window as any).PM.Kernel.services.get('timeline'), m = t.marquee;
    return { time: (m.x0 - t.gut) / t.pps + t.scrollT, value: t._graph.y2v(m.y0),
      x1: m.x1, y1: m.y1, scrollT: t.scrollT, min: t._graph.vmin };
  });
  const before = await read();
  for (const [dx, dy] of [[15, 8], [-15, -8]]) {
    const previous = await read();
    await page.mouse.wheel(dx, dy);
    await expect.poll(async () => (await read()).min).not.toBe(previous.min);
    await expect.poll(async () => (await read()).time).toBeCloseTo(before.time);
    await expect.poll(async () => (await read()).value).toBeCloseTo(before.value);
    const now = await read();
    expect(now.x1).toBeCloseTo(before.x1); expect(now.y1).toBeCloseTo(before.y1);
  }
  await page.mouse.up();
  expect(await page.evaluate(() => (window as any).PM.sel.keys.length)).toBe(2);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
