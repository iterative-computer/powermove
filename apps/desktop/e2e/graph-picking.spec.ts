import { expect, type Page } from '@playwright/test';
import { test } from './helpers/app';

async function graphFixture(page: Page) {
  await page.waitForFunction(() => Boolean((window as any).PM?.TL?.cv));
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.replaceProject(PM.mkProject({ name: 'Graph picking QA', dur: 6 }));
    const L = PM.mkLayer('text', { name: 'Introducing', dur: 6, d: { 'fontAxis.wght': PM.P(180) } });
    PM.proj.layers.push(L);
    for (const [path, values] of [['scale.x', [40, 100]], ['scale.y', [40, 100]], ['c.fontAxis.wght', [180, 560]]] as any[]) {
      const prop = PM.findProp(L, path);
      PM.setKeyOn(prop, 1, values[0]); PM.setKeyOn(prop, 3, values[1]);
    }
    PM.selectLayers(L.id); PM.sel.chan = 'scale';
    PM.TL.graphFocus = { layerId: L.id, trackKey: 'scale' };
    PM.TL.pps = 90; PM.TL.scrollT = 0;
    PM.sel.keys = PM.findProp(L, 'c.fontAxis.wght').kf.map((k: any) => k.i);
    PM.bus.emit('layers'); PM.invalidate();
  });
  await page.getByRole('button', { name: 'Graph editor (Shift+F3)', exact: true }).click();
  await page.waitForFunction(() => (window as any).PM.TL._graph?.series?.length >= 1);
}

test('a Weight handle edits Weight while Scale is focused, and Undo restores it', async ({ session }) => {
  const { page } = session;
  await graphFixture(page);
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.sel.keys = PM.allProps(PM.proj.layers[0]).flatMap((axis: any) => axis.prop.kf.map((key: any) => key.i));
    PM.bus.emit('sel'); PM.invalidate();
  });
  await page.waitForFunction(() => (window as any).PM.TL._graph?.series?.length === 3);
  const before = await page.evaluate(() => {
    const PM = (window as any).PM, L = PM.proj.layers[0], key = PM.findProp(L, 'c.fontAxis.wght').kf[0];
    const box = PM.TL.cv.getBoundingClientRect(), p = PM.UIState.getKeyHandles(key).ho;
    PM.hist.clear();
    return { scale: JSON.stringify([L.p['scale.x'], L.p['scale.y']]), weight: JSON.stringify(PM.findProp(L, 'c.fontAxis.wght')), x: box.x + p[0], y: box.y + p[1] };
  });
  await page.mouse.move(before.x, before.y); await page.mouse.down();
  await page.mouse.move(before.x + 16, before.y - 18, { steps: 6 }); await page.mouse.up();
  const state = () => page.evaluate(() => {
    const PM = (window as any).PM, L = PM.proj.layers[0];
    return { scale: JSON.stringify([L.p['scale.x'], L.p['scale.y']]), weight: JSON.stringify(PM.findProp(L, 'c.fontAxis.wght')) };
  });
  expect((await state()).weight).not.toBe(before.weight);
  expect((await state()).scale).toBe(before.scale);
  await page.evaluate(() => (window as any).PM.hist.undo());
  expect(await state()).toEqual({ scale: before.scale, weight: before.weight });
  expect(session.diagnostics.pageErrors).toEqual([]);
});

for (const side of ['ho', 'hi'] as const) test(`Shift snaps the ${side} handle flat and cancel restores native easing`, async ({ session }) => {
  const { page } = session;
  await graphFixture(page);
  const before = await page.evaluate(side => {
    const PM = (window as any).PM, prop = PM.findProp(PM.proj.layers[0], 'c.fontAxis.wght');
    const box = PM.TL.cv.getBoundingClientRect(), p = PM.UIState.getKeyHandles(prop.kf[side === 'ho' ? 0 : 1])[side];
    return { x: box.x + p[0], y: box.y + p[1], source: JSON.stringify(prop) };
  }, side);
  await page.mouse.move(before.x, before.y); await page.mouse.down();
  await page.keyboard.down('Shift');
  await page.mouse.move(before.x + 15, before.y - 18, { steps: 6 });
  const difference = await page.evaluate(side => {
    const PM = (window as any).PM, prop = PM.findProp(PM.proj.layers[0], 'c.fontAxis.wght');
    const key = prop.kf[side === 'ho' ? 0 : 1], p = PM.UIState.getKeyHandles(key);
    return { pixels: p[side][1] - p.pt[1], speed: key[side === 'ho' ? 'outEase' : 'inEase'].speed };
  }, side);
  expect(difference.pixels).toBeCloseTo(0); expect(difference.speed).toBe(0);
  await page.evaluate(() => window.dispatchEvent(new PointerEvent('pointercancel', { bubbles: true })));
  await page.mouse.up(); await page.keyboard.up('Shift');
  expect(await page.evaluate(() => {
    const PM = (window as any).PM;
    return JSON.stringify(PM.findProp(PM.proj.layers[0], 'c.fontAxis.wght'));
  })).toBe(before.source);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('vertical scrolling pans the graph without scrolling rows or editing animation', async ({ session }, info) => {
  const { page } = session;
  await graphFixture(page);
  const before = await page.evaluate(() => {
    const PM = (window as any).PM, T = PM.TL, box = T.cv.getBoundingClientRect();
    return { value: T._graph.vmin, rows: T.scrollY, source: JSON.stringify(PM.proj.layers), x: box.x + T.gut + 120, y: box.y + T.ruler + 80 };
  });
  await page.mouse.move(before.x, before.y); await page.mouse.wheel(0, 60);
  await expect.poll(() => page.evaluate(() => (window as any).PM.TL._graph.vmin)).toBeLessThan(before.value);
  expect(await page.evaluate(() => (window as any).PM.TL.scrollY)).toBe(before.rows);
  await page.mouse.wheel(0, -60);
  await expect.poll(() => page.evaluate(() => (window as any).PM.TL._graph.vmin)).toBeCloseTo(before.value);
  expect(await page.evaluate(() => JSON.stringify((window as any).PM.proj.layers))).toBe(before.source);
  await page.locator('#panel-timeline').screenshot({ path: info.outputPath('graph-cleanup.png') });
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('only selected keys and their curves appear, and clearing selection clears hit targets', async ({ session }, info) => {
  const { page } = session;
  await graphFixture(page);
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.sel.keys = [PM.findProp(PM.proj.layers[0], 'c.fontAxis.wght').kf[0].i];
    PM.bus.emit('sel'); PM.invalidate();
  });
  await expect.poll(() => page.evaluate(() => (window as any).PM.TL._graph?.points.length)).toBe(1);
  expect(await page.evaluate(() => (window as any).PM.TL._graph.series.map((axis: any) => axis.key))).toEqual(['c.fontAxis.wght']);
  await page.evaluate(() => {
    const PM = (window as any).PM, L = PM.proj.layers[0];
    PM.sel.keys = [...L.p['scale.x'].kf, ...L.p['scale.y'].kf].map((key: any) => key.i);
    PM.theme.apply('dark'); PM.bus.emit('sel'); PM.invalidate();
  });
  await expect.poll(() => page.evaluate(() => (window as any).PM.TL._graph?.series.map((axis: any) => axis.key))).toEqual(['scale.x', 'scale.y']);
  await page.locator('#panel-timeline').screenshot({ path: info.outputPath('selected-scale-dark.png') });
  await page.evaluate(() => { const PM = (window as any).PM; PM.sel.keys = []; PM.bus.emit('sel'); PM.invalidate(); });
  await expect.poll(() => page.evaluate(() => (window as any).PM.TL._graph)).toBeNull();
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('marquee keeps its original graph targets and selected points still drag together', async ({ session }) => {
  const { page } = session;
  await graphFixture(page);
  const box = await page.evaluate(() => {
    const PM = (window as any).PM, T = PM.TL, bounds = T.cv.getBoundingClientRect(), p = T._graph.points;
    return { x0: bounds.x + Math.min(...p.map((p: any) => p.x)) - 12, x1: bounds.x + Math.max(...p.map((p: any) => p.x)) + 12,
      y0: bounds.y + Math.min(...p.map((p: any) => p.y)) - 12, y1: bounds.y + Math.max(...p.map((p: any) => p.y)) + 12 };
  });
  await page.mouse.move(box.x0, box.y0); await page.mouse.down();
  await page.mouse.move(box.x1, box.y1, { steps: 12 }); await page.mouse.up();
  expect(await page.evaluate(() => (window as any).PM.sel.keys.length)).toBe(2);
  const before = await page.evaluate(() => {
    const PM = (window as any).PM, T = PM.TL, bounds = T.cv.getBoundingClientRect(), p = T._graph.points[0];
    PM.hist.clear();
    return { x: bounds.x + p.x, y: bounds.y + p.y, values: T._graph.points.map((p: any) => [p.key.t, p.key.v]) };
  });
  await page.mouse.move(before.x, before.y); await page.mouse.down();
  await page.mouse.move(before.x + 15, before.y - 10, { steps: 6 }); await page.mouse.up();
  const values = () => page.evaluate(() => (window as any).PM.TL._graph.points.map((p: any) => [p.key.t, p.key.v]));
  expect((await values()).every((p: number[], i: number) => p[0]! > before.values[i][0] && p[1]! > before.values[i][1])).toBe(true);
  await page.evaluate(() => (window as any).PM.hist.undo());
  await expect.poll(values).toEqual(before.values);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('drawing an empty graph marquee preserves selected keyframes', async ({ session }) => {
  const { page } = session;
  await graphFixture(page);
  const before = await page.evaluate(() => {
    const PM = (window as any).PM, T = PM.TL, box = T.cv.getBoundingClientRect();
    return { ids: [...PM.sel.keys], x: box.x + T.gut + 15, y: box.y + T.ruler + 45 };
  });
  await page.mouse.move(before.x, before.y); await page.mouse.down();
  await page.mouse.move(before.x + 25, before.y + 25, { steps: 4 });
  expect(await page.evaluate(() => Boolean((window as any).PM.TL.marquee))).toBe(true);
  expect(await page.evaluate(() => (window as any).PM.sel.keys)).toEqual(before.ids);
  await page.mouse.up();
  expect(await page.evaluate(() => (window as any).PM.sel.keys)).toEqual(before.ids);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('a graph marquee selects a subset and moves only enclosed points', async ({ session }) => {
  const { page } = session;
  await graphFixture(page);
  const target = await page.evaluate(() => {
    const PM = (window as any).PM, T = PM.TL, box = T.cv.getBoundingClientRect();
    const p = T._graph.points.reduce((a: any, b: any) => a.y > b.y ? a : b);
    return { id: p.key.i, x: box.x + p.x, y: box.y + p.y,
      keys: PM.findProp(PM.proj.layers[0], 'c.fontAxis.wght').kf.map((k: any) => ({ id: k.i, t: k.t, v: k.v })) };
  });
  await page.mouse.move(target.x - 14, target.y - 14); await page.mouse.down();
  await page.mouse.move(target.x + 14, target.y + 14, { steps: 6 }); await page.mouse.up();
  expect(await page.evaluate(() => (window as any).PM.sel.keys)).toEqual([target.id]);
  const point = await page.evaluate(() => {
    const T = (window as any).PM.TL, box = T.cv.getBoundingClientRect(), p = T._graph.points[0];
    return { x: box.x + p.x, y: box.y + p.y };
  });
  await page.mouse.move(point.x, point.y); await page.mouse.down();
  await page.mouse.move(point.x + 18, point.y - 10, { steps: 6 }); await page.mouse.up();
  const after = await page.evaluate(() => {
    const PM = (window as any).PM;
    return PM.findProp(PM.proj.layers[0], 'c.fontAxis.wght').kf.map((k: any) => ({ id: k.i, t: k.t, v: k.v }));
  });
  expect(after.find((k: any) => k.id === target.id)).not.toEqual(target.keys.find((k: any) => k.id === target.id));
  expect(after.filter((k: any) => k.id !== target.id)).toEqual(target.keys.filter((k: any) => k.id !== target.id));
  expect(session.diagnostics.pageErrors).toEqual([]);
});
