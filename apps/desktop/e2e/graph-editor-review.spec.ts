import { chooseNativeMenu, expect, test } from './helpers/app';

test('graph editor exposes its state and keeps value and speed navigation usable', async ({ session }, info) => {
  await session.openEditor();
  const { page } = session;
  const viewerPixel = () => page.evaluate(() => {
    const PM = (window as any).PM, gl = PM.GL.gl, pixel = new Uint8Array(4);
    gl.readPixels(Math.floor(gl.drawingBufferWidth / 2), Math.floor(gl.drawingBufferHeight / 2), 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, pixel);
    return Array.from(pixel);
  });
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.waitForFunction(() => Boolean((window as any).PM.Kernel.services.get('timeline').cv));
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
    PM.Kernel.services.get('timeline').reveal(layer, ['scale.x', 'scale.y']);
    PM.sel.keys = ['scale.x', 'scale.y'].flatMap(path => layer.p[path].kf.map((key: any) => key.i));
    PM.Kernel.services.get('timeline').keySelectionActive = true; PM.Kernel.services.get('timeline').pps = 90; PM.Kernel.services.get('timeline').scrollT = 0;
    PM.setTime(2); PM.hist.clear(); PM.invalidate();
  });
  const graph = page.getByRole('button', { name: 'Graph editor (Shift+F3)', exact: true });
  await graph.click();
  await page.waitForFunction(() => (window as any).PM.Kernel.services.get('timeline')._graph?.series.length === 2);
  await page.screenshot({ path: info.outputPath('value-graph.png') });
  await expect(graph).toHaveAttribute('aria-pressed', 'true');
  const initialPixel = await viewerPixel();
  expect(initialPixel).toEqual([96, 123, 255, 255]);
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
  await chooseNativeMenu(session, 'Speed graph', () => options.click());
  await expect.poll(() => page.evaluate(() => (window as any).PM.Kernel.services.get('timeline').graphType)).toBe('speed');
  await page.screenshot({ path: info.outputPath('speed-graph.png') });
  const canvas = await page.locator('#tl-canvas').boundingBox();
  const gutter = await page.evaluate(() => (window as any).PM.Kernel.services.get('timeline').gut);
  await page.mouse.move(canvas!.x + gutter + 100, canvas!.y + 160);
  await page.mouse.wheel(0, 150);
  await expect.poll(() => page.evaluate(() => Boolean((window as any).PM.Kernel.services.get('timeline').graphViewBounds))).toBe(true);
  await chooseNativeMenu(session, 'Fit selected curves', () => options.click());
  await expect.poll(() => page.evaluate(() => (window as any).PM.Kernel.services.get('timeline').graphViewBounds)).toBeNull();
  await page.evaluate(() => new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
  const before = await page.evaluate(() => (window as any).PM.proj.layers[0].p['scale.x'].kf.map((key: any) => ({ t: key.t, v: key.v, speed: key.outEase.speed })));
  const point = await page.evaluate(() => {
    const PM = (window as any).PM, key = PM.proj.layers[0].p['scale.x'].kf[1], b = PM.Kernel.services.get('timeline').cv.getBoundingClientRect();
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
  await chooseNativeMenu(session, 'Value graph', () => options.click());
  await page.screenshot({ path: info.outputPath('value-graph-fitted.png') });
  console.log('GRAPH_SAMPLES', samples);
  expect(await viewerPixel()).toEqual(initialPixel);
  await page.locator('.panel[data-panel="timeline"]').screenshot({ path: info.outputPath('graph-panel.png') });
  expect(session.diagnostics.pageErrors).toEqual([]);
});
