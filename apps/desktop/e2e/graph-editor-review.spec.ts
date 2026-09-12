import { expect, test } from './helpers/app';

test('graph editor exposes its state and keeps value and speed navigation usable', async ({ session }, info) => {
  const { page } = session;
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.waitForFunction(() => Boolean((window as any).PM.TL.cv));
  await page.evaluate(() => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ name: 'Graph editor review', w: 1280, h: 720, dur: 8 });
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: project }));
    PM.ProjectsScreen.hide();
    const layer = PM.mkLayer('shape', { name: 'Card', d: { w: 220, h: 140, color: '#607BFF', radius: 18 } });
    PM.proj.layers = [layer]; PM.ProjectIndex.invalidate();
    for (const [path, values] of [['scale.x', [100, 220, 140]], ['scale.y', [70, 160, 70]]] as const) {
      PM.animate(layer, path, values.map((v, i) => ({ t: 1 + i * 2, v })), { ease: 'power' });
    }
    PM.selectLayers(layer.id); PM.sel.chan = 'scale';
    PM.TL.reveal(layer, ['scale.x', 'scale.y']);
    PM.sel.keys = ['scale.x', 'scale.y'].flatMap(path => layer.p[path].kf.map((key: any) => key.i));
    PM.TL.keySelectionActive = true; PM.TL.pps = 90; PM.TL.scrollT = 0;
    PM.setTime(2); PM.hist.clear(); PM.invalidate();
  });
  const graph = page.getByRole('button', { name: 'Graph editor (Shift+F3)', exact: true });
  await graph.click();
  await page.waitForFunction(() => (window as any).PM.TL._graph?.series.length === 2);
  await page.screenshot({ path: info.outputPath('value-graph.png') });
  await expect(graph).toHaveAttribute('aria-pressed', 'true');
  await page.keyboard.press('Shift+F3');
  await expect(graph).toHaveAttribute('aria-pressed', 'false');
  await page.keyboard.press('Shift+F3');
  await expect(graph).toHaveAttribute('aria-pressed', 'true');
  const samples = await page.evaluate(() => {
    const PM = (window as any).PM, original = PM.evP;
    let calls = 0;
    PM.evP = (...args: any[]) => { calls++; return original(...args); };
    try {
      PM.touch(); PM.bus.emit('draw:timeline'); const cold = calls;
      calls = 0; PM.bus.emit('draw:timeline');
      return { cold, warm: calls };
    } finally { PM.evP = original; PM.invalidate(); }
  });
  expect(samples.cold).toBeGreaterThan(100);
  expect(samples.warm).toBeLessThan(samples.cold / 4);
  const options = page.getByRole('button', { name: 'Graph options', exact: true });
  await options.click();
  await page.getByRole('menuitem', { name: 'Speed graph', exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).PM.TL.graphType)).toBe('speed');
  await page.screenshot({ path: info.outputPath('speed-graph.png') });
  const canvas = await page.locator('#tl-canvas').boundingBox();
  const gutter = await page.evaluate(() => (window as any).PM.TL.gut);
  await page.mouse.move(canvas!.x + gutter + 100, canvas!.y + 160);
  await page.mouse.wheel(0, 150);
  await expect.poll(() => page.evaluate(() => Boolean((window as any).PM.TL.graphViewBounds))).toBe(true);
  await options.click();
  await page.getByRole('menuitem', { name: 'Fit selected curves', exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).PM.TL.graphViewBounds)).toBeNull();
  const before = await page.evaluate(() => (window as any).PM.proj.layers[0].p['scale.x'].kf.map((key: any) => ({ t: key.t, v: key.v, speed: key.outEase.speed })));
  const point = await page.evaluate(() => {
    const PM = (window as any).PM, key = PM.proj.layers[0].p['scale.x'].kf[1], b = PM.TL.cv.getBoundingClientRect();
    const point = PM.UIState.getKeyHandles(key).pt;
    return { x: b.x + point[0], y: b.y + point[1] };
  });
  await page.mouse.move(point.x, point.y); await page.mouse.down();
  await page.mouse.move(point.x, point.y - 12, { steps: 5 }); await page.mouse.up();
  const after = await page.evaluate(() => (window as any).PM.proj.layers[0].p['scale.x'].kf.map((key: any) => ({ t: key.t, v: key.v, speed: key.outEase.speed })));
  expect(after.map(({ t, v }: any) => ({ t, v }))).toEqual(before.map(({ t, v }: any) => ({ t, v })));
  expect(after.map((key: any) => key.speed)).not.toEqual(before.map((key: any) => key.speed));
  await page.keyboard.press('Meta+z');
  expect(await page.evaluate(() => (window as any).PM.proj.layers[0].p['scale.x'].kf.map((key: any) => ({ t: key.t, v: key.v, speed: key.outEase.speed })))).toEqual(before);
  await options.click();
  await page.getByRole('menuitem', { name: 'Value graph', exact: true }).click();
  await page.screenshot({ path: info.outputPath('value-graph-fitted.png') });
  console.log('GRAPH_SAMPLES', samples);
  console.log('GRAPH_FINAL', await page.evaluate(() => { const PM = (window as any).PM, L = PM.proj.layers[0]; return { time: PM.time, on: L.on, active: PM.active(L, PM.time), matrix: PM.worldMatrix(L, PM.time), draws: PM.GL.stats.draws, dirty: PM.dirty, selected: PM.sel.layers }; }));
  expect(session.diagnostics.pageErrors).toEqual([]);
});
