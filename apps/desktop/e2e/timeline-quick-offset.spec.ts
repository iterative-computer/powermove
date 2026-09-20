import { expect, test } from './helpers/app';

async function waitForTimeline(page: any) {
  await page.waitForFunction(() => Boolean((window as any).PM?.Kernel?.services?.get('timeline')?.cv && (window as any).PM?.Edit));
}

test.describe('@timeline Quick Offset', () => {
  test('stagger layers in selection order with a subframe Cmd+Option drag and one undo', async ({ session }) => {
  await session.openEditor();
    const { page } = session;
    await waitForTimeline(page);
    const ids = await page.evaluate(() => {
      const PM = (window as any).PM;
      const project = PM.mkProject({ name: 'Quick Offset layers', w: 640, h: 360, fps: 30, dur: 8 });
      const layers = ['A', 'B', 'C'].map(name => PM.mkLayer('shape', {
        name, from: 1, dur: 4,
        d: { shape: 'rect', color: '#ffffff', w: 80, h: 80, radius: 0, stroke: 0, strokeColor: '#000000', points: 5 },
      }, project));
      project.layers = layers;
      PM.replaceProject(project);
      PM.selectLayers([layers[1].id, layers[0].id, layers[2].id]);
      PM.Kernel.services.get('timeline').pps = 100;
      PM.Kernel.services.get('timeline').scrollT = 0;
      PM.bus.emit('layers'); PM.invalidate('timeline');
      return layers.map(layer => layer.id);
    });
    await expect.poll(() => page.evaluate(() => (window as any).PM.Kernel.services.get('timeline').rows
      .filter((item: any) => item.kind === 'layer').length)).toBe(3);
    const overlay = await page.evaluate(() => {
      const PM = (window as any).PM;
      const layers = PM.proj.layers;
      const row = PM.Kernel.services.get('timeline').rows.findIndex((item: any) => item.kind === 'layer' && item.L.id === layers[1].id);
      const rect = PM.Kernel.services.get('timeline').cv.getBoundingClientRect();
      const x = rect.left + PM.Kernel.services.get('timeline').gut + 150;
      const y = rect.top + PM.Kernel.services.get('timeline').ruler + row * PM.Kernel.services.get('timeline').row + PM.Kernel.services.get('timeline').row / 2;
      const event = (type: string, clientX: number) => new PointerEvent(type, {
        bubbles: true, cancelable: true, button: 0, buttons: type === 'pointerup' ? 0 : 1,
        clientX, clientY: y, pointerId: 71, metaKey: true, altKey: true,
      });
      const down = event('pointerdown', x);
      Object.defineProperties(down, { offsetX: { value: x - rect.left }, offsetY: { value: y - rect.top } });
      PM.Kernel.services.get('timeline').cv.dispatchEvent(down);
      window.dispatchEvent(event('pointermove', x + 125));
      const overlay = { ...PM.Kernel.services.get('timeline').quickOffset };
      window.dispatchEvent(event('pointerup', x + 125));
      return overlay;
    });
    expect(overlay).toMatchObject({ total: 1.25, perGroup: 0.625 });

    const staggered = await page.evaluate((ids: string[]) => {
      const PM = (window as any).PM;
      return ids.map(id => PM.L(id).from);
    }, ids);
    expect(staggered).toEqual([1.625, 1, 2.25]);
    expect(await page.evaluate(() => (window as any).PM.Kernel.services.get('timeline').quickOffset)).toBeNull();
    expect(await page.evaluate(() => (window as any).PM.hist.undo())).toBe(true);
    expect(await page.evaluate((ids: string[]) => ids.map(id => (window as any).PM.L(id).from), ids)).toEqual([1, 1, 1]);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('stagger selected keyframe groups per layer without changing each group span', async ({ session }) => {
  await session.openEditor();
    const { page } = session;
    await waitForTimeline(page);
    const setup = await page.evaluate(() => {
      const PM = (window as any).PM;
      const project = PM.mkProject({ name: 'Quick Offset keys', w: 640, h: 360, fps: 30, dur: 8 });
      const layers = ['A', 'B', 'C'].map(name => PM.mkLayer('shape', { name, dur: 8 }, project));
      project.layers = layers;
      for (const layer of layers) {
        PM.setKey(layer, 'opacity', 1, 0);
        PM.setKey(layer, 'opacity', 2.25, 100);
      }
      PM.replaceProject(project);
      layers.forEach(layer => PM.Kernel.services.get('timeline').reveal(layer, ['opacity']));
      PM.sel.layers = [layers[1].id, layers[0].id, layers[2].id];
      PM.sel.keys = layers.flatMap(layer => layer.p.opacity.kf.map((key: any) => key.i));
      PM.sel.chan = 'opacity'; PM.Kernel.services.get('timeline').keySelectionActive = true;
      PM.Kernel.services.get('timeline').pps = 100; PM.Kernel.services.get('timeline').scrollT = 0;
      PM.bus.emit('layers'); PM.bus.emit('sel'); PM.invalidate('timeline');
      return { ids: layers.map(layer => layer.id), before: layers.map(layer => layer.p.opacity.kf.map((key: any) => key.t)) };
    });
    await expect.poll(() => page.evaluate(() => (window as any).PM.Kernel.services.get('timeline').rows
      .filter((item: any) => item.kind === 'prop' && item.key === 'opacity').length)).toBe(3);
    await page.evaluate(() => {
      const PM = (window as any).PM, layers = PM.proj.layers;
      const row = PM.Kernel.services.get('timeline').rows.findIndex((item: any) => item.kind === 'prop' && item.L.id === layers[1].id && item.key === 'opacity');
      const rect = PM.Kernel.services.get('timeline').cv.getBoundingClientRect();
      const x = rect.left + PM.Kernel.services.get('timeline').gut + 100;
      const y = rect.top + PM.Kernel.services.get('timeline').ruler + row * PM.Kernel.services.get('timeline').row + PM.Kernel.services.get('timeline').row / 2;
      const event = (type: string, clientX: number) => new PointerEvent(type, {
        bubbles: true, cancelable: true, button: 0, buttons: type === 'pointerup' ? 0 : 1,
        clientX, clientY: y, pointerId: 72, metaKey: true, altKey: true,
      });
      const down = event('pointerdown', x);
      Object.defineProperties(down, { offsetX: { value: x - rect.left }, offsetY: { value: y - rect.top } });
      PM.Kernel.services.get('timeline').cv.dispatchEvent(down);
      window.dispatchEvent(event('pointermove', x + 125));
      window.dispatchEvent(event('pointerup', x + 125));
    });

    const times = await page.evaluate((ids: string[]) => ids.map(id =>
      (window as any).PM.L(id).p.opacity.kf.map((key: any) => key.t)), setup.ids);
    expect(times[0]![0]).toBeCloseTo(setup.before[0]![0]! + .625, 6);
    expect(times[1]).toEqual(setup.before[1]);
    expect(times[2]![0]).toBeCloseTo(setup.before[2]![0]! + 1.25, 6);
    expect(times.every((pair: number[], index: number) =>
      Math.abs((pair[1]! - pair[0]!) - (setup.before[index]![1]! - setup.before[index]![0]!)) < 1e-9)).toBe(true);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
});

test('one Graph Editor handle adjusts selected same-property keyframes across layers', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await waitForTimeline(page);
  const ids = await page.evaluate(() => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ name: 'Linked easing', w: 640, h: 360, fps: 30, dur: 6 });
    const layers = ['A', 'B'].map(name => PM.mkLayer('shape', { name, dur: 6 }, project));
    project.layers = layers;
    layers.forEach((layer: any, index: number) => {
      PM.setKey(layer, 'opacity', 1, index * 20);
      PM.setKey(layer, 'opacity', 4, 100 - index * 20);
    });
    PM.replaceProject(project);
    PM.sel.layers = layers.map(layer => layer.id);
    PM.sel.keys = layers.map(layer => layer.p.opacity.kf[0].i);
    PM.sel.chan = 'opacity'; PM.Kernel.services.get('timeline').keySelectionActive = true;
    PM.bus.emit('layers'); PM.bus.emit('sel'); PM.invalidate('timeline');
    return layers.map(layer => layer.id);
  });
  await page.evaluate(() => (document.querySelector('button[title="Graph editor (Shift+F3)"]') as HTMLButtonElement).click());
  await page.waitForFunction((layerIds: string[]) => {
    const PM = (window as any).PM;
    return layerIds.every(id => PM.UIState.getKeyHandles(PM.L(id).p.opacity.kf[0])?.ho);
  }, ids);
  await page.evaluate((id: string) => {
    const PM = (window as any).PM, rect = PM.Kernel.services.get('timeline').cv.getBoundingClientRect();
    const handle = PM.UIState.getKeyHandles(PM.L(id).p.opacity.kf[0]).ho;
    const x = rect.left + handle[0], y = rect.top + handle[1];
    const event = (type: string, clientX: number, clientY: number) => new PointerEvent(type, {
      bubbles: true, cancelable: true, button: 0, buttons: type === 'pointerup' ? 0 : 1,
      clientX, clientY, pointerId: 73,
    });
    const down = event('pointerdown', x, y);
    Object.defineProperties(down, { offsetX: { value: x - rect.left }, offsetY: { value: y - rect.top } });
    PM.Kernel.services.get('timeline').cv.dispatchEvent(down);
    window.dispatchEvent(event('pointermove', x + 16, y - 14));
    window.dispatchEvent(event('pointerup', x + 16, y - 14));
  }, ids[0]);

  const handles = await page.evaluate((layerIds: string[]) => layerIds.map(id =>
    (window as any).PM.L(id).p.opacity.kf[0].eo), ids);
  expect(handles[0]).not.toEqual([0, 0]);
  expect(handles[1]).toEqual(handles[0]);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
