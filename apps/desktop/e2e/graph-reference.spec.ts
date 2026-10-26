import { chooseNativeMenu, expect, test } from './helpers/app';

test('reference graph supports combined editing, persistent handles, and group scaling with undo', async ({ session }, info) => {
  await session.openEditor();
  const { page } = session;
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.evaluate(() => {
    const PM = (window as any).PM, timeline = PM.Kernel.services.get('timeline');
    PM.theme.apply('dark');
    PM.replaceProject(PM.mkProject({ name: 'Curve editor', dur: 8 }));
    const layer = PM.mkLayer('shape', { name: 'Motion', dur: 8, d: { w: 120, h: 120, color: '#f36870' } });
    PM.proj.layers.push(layer);
    PM.animate(layer, 'position.x', [{ t: 1, v: 0 }, { t: 2, v: 60 }, { t: 3, v: 230 }, { t: 5, v: 140 }, { t: 6, v: -15 }], { ease: 'easeInOut' });
    PM.animate(layer, 'position.y', [{ t: 1, v: -120 }, { t: 3, v: 160 }, { t: 4, v: 210 }, { t: 6, v: 195 }], { ease: 'easeInOut' });
    PM.applyEaseTo([...layer.p['position.x'].kf, ...layer.p['position.y'].kf], 'easeInOut');
    PM.selectLayers(layer.id); PM.sel.chan = 'position.x';
    PM.sel.keys = [...layer.p['position.x'].kf, ...layer.p['position.y'].kf].map((k: any) => k.i);
    timeline.reveal(layer, ['position.x', 'position.y']);
    timeline.pps = 100; timeline.scrollT = 0;
    PM.setTime(3.5); PM.bus.emit('layers'); PM.invalidate();
  });
  // One Graph Editor switch; "layer bars with the graph" is its menu option.
  const graph = page.getByRole('button', { name: 'Graph editor (Shift+F3)', exact: true });
  await page.evaluate(() => { (window as any).PM.Kernel.services.get('timeline').graphCombined = true; });
  await graph.click();
  await expect(graph).toHaveAttribute('aria-pressed', 'true');
  await page.waitForFunction(() => (window as any).PM.Kernel.services.get('timeline')._graph?.points.length === 9);
  const dopeKey = await page.evaluate(() => {
    const PM = (window as any).PM, T = PM.Kernel.services.get('timeline'), b = T.cv.getBoundingClientRect();
    const index = T.rows.findIndex((row: any) => row.key === 'position.x');
    PM.sel.keys = []; PM.hist.clear(); PM.bus.emit('sel'); PM.invalidate();
    return { x: b.x + T.gut + T.pps, y: b.y + T.ruler + (index + .5) * T.row - T.scrollY };
  });
  await page.mouse.move(dopeKey.x, dopeKey.y); await page.mouse.down();
  await page.mouse.move(dopeKey.x + 20, dopeKey.y, { steps: 5 }); await page.mouse.up();
  await expect.poll(() => page.evaluate(() => (window as any).PM.proj.layers[0].p['position.x'].kf[0].t)).toBeCloseTo(1.2);
  await page.evaluate(() => (window as any).PM.hist.undo());
  await expect.poll(() => page.evaluate(() => (window as any).PM.proj.layers[0].p['position.x'].kf[0].t)).toBe(1);
  const split = await page.evaluate(() => {
    const T = (window as any).PM.Kernel.services.get('timeline'), b = T.cv.getBoundingClientRect();
    return { x: b.x + T.gut + 100, y: b.y + T.ruler + (T.hgt - T.ruler) * .38 };
  });
  await page.mouse.move(split.x, split.y); await page.mouse.down();
  await page.mouse.move(split.x, split.y + 25, { steps: 5 }); await page.mouse.up();
  await expect.poll(() => page.evaluate(() => (window as any).PM.Kernel.services.get('timeline').graphSplit)).toBeGreaterThan(.38);
  await page.locator('#panel-timeline').screenshot({ path: info.outputPath('combined-graph.png') });
  await graph.click(); await expect(graph).toHaveAttribute('aria-pressed', 'false');
  await page.evaluate(() => { (window as any).PM.Kernel.services.get('timeline').graphCombined = false; });
  await graph.click(); await expect(graph).toHaveAttribute('aria-pressed', 'true');
  await chooseNativeMenu(session, 'Fit selected curves', () => page.getByRole('button', { name: 'Graph options', exact: true }).click());
  // Clear selection: the curves and curved-key tangents stay available.
  await page.evaluate(() => { const PM = (window as any).PM; PM.sel.keys = []; PM.bus.emit('sel'); PM.invalidate(); });
  await expect.poll(() => page.evaluate(() => (window as any).PM.Kernel.services.get('timeline')._graph?.points.length)).toBe(9);
  await page.waitForFunction(() => {
    const PM = (window as any).PM;
    return !!PM.UIState.getKeyHandles(PM.proj.layers[0].p['position.x'].kf[1])?.ho;
  });
  const handle = await page.evaluate(() => {
    const PM = (window as any).PM, T = PM.Kernel.services.get('timeline'), b = T.cv.getBoundingClientRect();
    const prop = PM.proj.layers[0].p['position.x'], key = prop.kf[1], pt = PM.UIState.getKeyHandles(key).ho;
    PM.hist.clear();
    return { x: b.x + pt[0], y: b.y + pt[1], before: JSON.stringify(prop) };
  });
  await page.mouse.move(handle.x, handle.y); await page.mouse.down();
  await page.mouse.move(handle.x + 18, handle.y - 16, { steps: 6 }); await page.mouse.up();
  const curve = () => page.evaluate(() => JSON.stringify((window as any).PM.proj.layers[0].p['position.x']));
  expect(await curve()).not.toBe(handle.before);
  await page.evaluate(() => (window as any).PM.hist.undo());
  expect(await curve()).toBe(handle.before);
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.sel.keys = PM.proj.layers[0].p['position.x'].kf.slice(1, 4).map((k: any) => k.i);
    PM.bus.emit('sel'); PM.invalidate(); PM.hist.clear();
  });
  await page.waitForFunction(() => !!(window as any).PM.Kernel.services.get('timeline')._graph?.transform);
  const grip = await page.evaluate(() => {
    const PM = (window as any).PM, T = PM.Kernel.services.get('timeline'), b = T.cv.getBoundingClientRect(), box = T._graph.selectionBounds;
    return { x: b.x + box.x1, y: b.y + box.y0, before: JSON.stringify(PM.proj.layers[0].p['position.x']) };
  });
  await page.mouse.move(grip.x, grip.y); await page.mouse.down();
  await page.mouse.move(grip.x + 25, grip.y - 14, { steps: 6 }); await page.mouse.up();
  expect(await curve()).not.toBe(grip.before);
  await page.locator('#panel-timeline').screenshot({ path: info.outputPath('reference-graph.png') });
  await page.evaluate(() => (window as any).PM.hist.undo());
  expect(await curve()).toBe(grip.before);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
