import { expect } from '@playwright/test';
import { test } from './helpers/app';
import type { Page } from '@playwright/test';

async function scaleFixture(page: Page) {
  await page.waitForFunction(() => { const PM = (window as any).PM; const timeline = PM.Kernel.services.get('timeline'); return Boolean(timeline?.cv); });
  return page.evaluate(() => {
    const PM = (window as any).PM;
    const timeline = PM.Kernel.services.get('timeline');
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: PM.mkProject({ name: 'Timeline QA', dur: 10 }) })); PM.ProjectsScreen.hide();
    const layer = PM.mkLayer('solid', { name: 'Scale QA', from: 2, dur: 3 });
    PM.proj.layers = [...PM.proj.layers, layer];
    layer.scaleLinked = false;
    PM.setKey(layer, 'scale.x', 3, 100);
    PM.setKey(layer, 'scale.x', 4, 200);
    PM.setKey(layer, 'scale.y', 3, 50);
    PM.setKey(layer, 'scale.y', 4, 100);
    PM.selectLayers(layer.id);
    PM.sel.chan = 'scale';
    timeline.pps = 45;
    timeline.scrollT = 0;
    timeline.reveal(layer, ['scale.x', 'scale.y']);
    PM.setTime(3);
    PM.hist.clear();
    PM.invalidate();
    return layer.id;
  });
}

async function scaleKeyPoint(page: Page, time: number) {
  await expect.poll(() => page.evaluate(() => { const PM = (window as any).PM; const timeline = PM.Kernel.services.get('timeline'); return timeline.rows.filter((row: any) => row.key === 'scale').length; })).toBe(1);
  return page.evaluate((time) => {
    const PM = (window as any).PM;
    const timeline = PM.Kernel.services.get('timeline');
    const box = timeline.cv.getBoundingClientRect();
    const index = timeline.rows.findIndex((row: any) => row.key === 'scale');
    return { x: box.x + timeline.gut + (time - timeline.scrollT) * timeline.pps,
      y: box.y + timeline.ruler + index * timeline.row - timeline.scrollY + timeline.row / 2, pps: timeline.pps };
  }, time);
}

async function openGraph(page: Page) {
  await page.evaluate(() => {
    const PM = (window as any).PM;
    const timeline = PM.Kernel.services.get('timeline');
    if (!PM.sel.keys.length) {
      PM.sel.keys = PM.selLayers().flatMap((layer: any) => PM.allProps(layer).flatMap(({ prop }: any) => prop.kf.map((key: any) => key.i)));
      timeline.keySelectionActive = true;
    }
    PM.bus.emit('sel'); PM.invalidate('timeline');
  });
  await page.getByRole('button', { name: 'Graph editor (Shift+F3)', exact: true }).click();
}

test('Scale keys move outside the layer, delete together, and never delete their layer', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  const id = await scaleFixture(page);
  const point = await scaleKeyPoint(page, 3);
  const rows = await page.evaluate(() => { const PM = (window as any).PM; const timeline = PM.Kernel.services.get('timeline'); return timeline.rows.map((row: any) => row.key).filter(Boolean); });
  expect(rows).toEqual(['scale']);
  await page.mouse.move(point.x, point.y);
  await page.mouse.down();
  await page.mouse.move(point.x - 2 * point.pps, point.y, { steps: 8 });
  await page.mouse.up();
  expect(await page.evaluate((id) => {
    const PM = (window as any).PM, layer = PM.L(id);
    return { keys: PM.sel.keys.length, from: layer.from, dur: layer.dur,
      times: [layer.p['scale.x'].kf[0].t, layer.p['scale.y'].kf[0].t] };
  }, id)).toEqual({ keys: 2, from: 2, dur: 3, times: [-1, -1] });
  const later = await scaleKeyPoint(page, 4);
  await page.mouse.move(later.x, later.y);
  await page.mouse.down();
  await page.mouse.move(later.x + 2 * later.pps, later.y, { steps: 8 });
  await page.mouse.up();
  expect(await page.evaluate((id) => (window as any).PM.L(id).p['scale.x'].kf[1].t, id)).toBe(4);
  await page.keyboard.press('Backspace');
  await page.keyboard.press('Backspace');
  expect(await page.evaluate((id) => {
    const PM = (window as any).PM;
    return [PM.proj.layers.length, PM.L(id).p['scale.x'].kf.length, PM.L(id).p['scale.y'].kf.length];
  }, id)).toEqual([1, 1, 1]);
  await page.keyboard.press('Meta+z');
  expect(await page.evaluate((id) => (window as any).PM.L(id).p['scale.y'].kf.length, id)).toBe(2);
  await page.evaluate(async () => {
    const PM = (window as any).PM;
    // replaceProject sets fixture data; this window already owns that document,
    // so the relaunch below restores it rather than the welcome demo.
    await PM.flushProject();
  });
  await session.relaunch();
  await session.openEditor();
  expect(await session.page.evaluate((id) => {
    const L = (window as any).PM.L(id);
    return [L.p['scale.x'].kf[0].t, L.p['scale.y'].kf[0].t];
  }, id)).toEqual([-1, -1]);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('the easing grid applies a curve to both Scale dimensions and preserves linear defaults', async ({ session }, testInfo) => {
  await session.openEditor();
  const { page } = session;
  const id = await scaleFixture(page);
  expect(await page.evaluate((id) => (window as any).PM.L(id).p['scale.x'].kf[0].eo, id)).toEqual([0, 0]);
  const point = await scaleKeyPoint(page, 3);
  await page.mouse.click(point.x, point.y, { button: 'right' });
  const grid = page.locator('[role="menu"].curve-grid');
  await expect(grid.locator('.curve-option')).toHaveCount(8);
  await expect(grid.locator('.curve-line')).toHaveCount(8);
  const boxes = await grid.locator('.curve-option').evaluateAll(elements => elements.map(el => {
    const b = el.getBoundingClientRect(); return { x: b.x, y: b.y };
  }));
  expect(boxes[0]!.y).toBe(boxes[3]!.y);
  expect(boxes[4]!.y).toBeGreaterThan(boxes[0]!.y);
  expect(boxes[4]!.x).toBe(boxes[0]!.x);
  await testInfo.attach('easing-grid', { body: await grid.screenshot(), contentType: 'image/png' });
  await grid.getByRole('menuitem', { name: 'Ease out', exact: true }).click();
  expect(await page.evaluate((id) => {
    const L = (window as any).PM.L(id);
    return [L.p['scale.x'].kf[0].ei, L.p['scale.y'].kf[0].ei];
  }, id)).toEqual([[.58, 1], [.58, 1]]);
  await page.keyboard.press('Meta+z');
  expect(await page.evaluate((id) => (window as any).PM.L(id).p['scale.y'].kf[0].ei, id)).toEqual([1, 1]);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('the disclosure opens and collapses the selected layer strip', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  const id = await scaleFixture(page);
  await expect.poll(() => page.evaluate((id) => {
    const PM = (window as any).PM;
    return PM.UIState.getLayerCollapsed(PM.L(id));
  }, id)).toBe(false);

  const disclosure = await page.evaluate(() => { const PM = (window as any).PM, timeline = PM.Kernel.services.get('timeline'), b = timeline.cv.getBoundingClientRect(); return { x: b.x + 64, y: b.y + timeline.ruler + timeline.row / 2 }; });
  await page.mouse.click(disclosure.x, disclosure.y);
  await expect.poll(() => page.evaluate((id) => {
    const PM = (window as any).PM;
    return PM.UIState.getLayerCollapsed(PM.L(id));
  }, id)).toBe(true);

  await page.mouse.click(disclosure.x, disclosure.y);
  await expect.poll(() => page.evaluate((id) => {
    const PM = (window as any).PM;
    return PM.UIState.getLayerCollapsed(PM.L(id));
  }, id)).toBe(false);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('Command Shift D selects the new segment to the right of the playhead', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  const leftId = await scaleFixture(page);
  const animation = () => page.evaluate(() => {
    const PM = (window as any).PM, layer = PM.selLayers()[0] ?? PM.proj.layers[0];
    return PM.allProps(layer).map(({ prop }: any) => prop.kf.map((k: any) => ({ t: k.t + layer.from, v: k.v })));
  });
  const before = await animation();
  await page.keyboard.press('Meta+Shift+d');
  expect(await animation()).toEqual(before);

  expect(await page.evaluate((leftId) => {
    const PM = (window as any).PM;
    const selected = PM.selLayers();
    return {
      selectedIds: PM.sel.layers,
      selected: selected.map((layer: any) => ({ id: layer.id, from: layer.from, dur: layer.dur })),
      left: { id: PM.L(leftId).id, from: PM.L(leftId).from, dur: PM.L(leftId).dur }
    };
  }, leftId)).toEqual({
    selectedIds: expect.arrayContaining([expect.any(String)]),
    selected: [{ id: expect.any(String), from: 3, dur: 2 }],
    left: { id: leftId, from: 2, dur: 1 }
  });
  expect(await page.evaluate(() => {
    const PM = (window as any).PM;
    return PM.sel.layers.length === 1 && PM.sel.layers[0] !== PM.proj.layers.find((layer: any) => layer.from === 2)?.id;
  })).toBe(true);
  await page.keyboard.press('Meta+z');
  expect(await animation()).toEqual(before);
  expect(await page.evaluate(() => (window as any).PM.proj.layers.length)).toBe(1);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

for (const descending of [false, true]) test(`incoming Bézier handles follow the pointer, undo and cancel (${descending ? 'falling' : 'rising'})`, async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  const id = await scaleFixture(page);
  if (descending) await page.evaluate((id) => {
    const PM = (window as any).PM;
    PM.L(id).p['scale.x'].kf[1].v = 0;
    PM.L(id).p['scale.y'].kf[1].v = 0;
    PM.touch(); PM.invalidate();
  }, id);
  await openGraph(page);
  await page.waitForFunction((id) => {
    const PM = (window as any).PM;
    const timeline = PM.Kernel.services.get('timeline');
    return timeline._graph?.series?.length === 2 && PM.UIState.getKeyHandles(PM.L(id).p['scale.x'].kf[1])?.hi;
  }, id);
  const point = await page.evaluate((id) => {
    const PM = (window as any).PM;
    const timeline = PM.Kernel.services.get('timeline');
    const box = timeline.cv.getBoundingClientRect();
    const pt = PM.UIState.getKeyHandles(PM.L(id).p['scale.x'].kf[1]).hi;
    return { x: box.x + pt[0], y: box.y + pt[1] };
  }, id);
  await page.mouse.move(point.x, point.y);
  await page.mouse.down();
  await page.mouse.move(point.x + 8, point.y - 12, { steps: 8 });
  await page.mouse.up();
  const result = await page.evaluate((id) => {
    const PM = (window as any).PM, L = PM.L(id);
    return [L.p['scale.x'].kf[1].ei, L.p['scale.y'].kf[1].ei];
  }, id);
  expect(result[0][0]).toBeGreaterThan(2 / 3);
  if (descending) expect(result[0][1]).toBeLessThan(2 / 3);
  else expect(result[0][1]).toBeGreaterThan(2 / 3);
  expect(result[1]).toEqual(result[0]);
  await page.keyboard.press('Meta+z');
  expect(await page.evaluate((id) => (window as any).PM.L(id).p['scale.x'].kf[1].ei, id)).toEqual([1, 1]);
  await page.mouse.move(point.x, point.y);
  await page.mouse.down();
  await page.mouse.move(point.x + 8, point.y - 12, { steps: 4 });
  await page.evaluate(() => window.dispatchEvent(new PointerEvent('pointercancel', { bubbles: true })));
  await page.mouse.up();
  expect(await page.evaluate((id) => (window as any).PM.L(id).p['scale.x'].kf[1].ei, id)).toEqual([1, 1]);
  expect(await page.evaluate((id) => (window as any).PM.L(id).p['scale.y'].kf[1].ei, id)).toEqual([1, 1]);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('editing one Bézier handle keeps the neighboring handle visible', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  const id = await scaleFixture(page);
  await openGraph(page);
  await page.waitForFunction((layerId) => {
    const PM = (window as any).PM;
    return Boolean(PM.UIState.getKeyHandles(PM.L(layerId).p['scale.x'].kf[1])?.hi);
  }, id);
  const incoming = await page.evaluate((layerId) => {
    const PM = (window as any).PM;
    const timeline = PM.Kernel.services.get('timeline');
    const box = timeline.cv.getBoundingClientRect();
    const point = PM.UIState.getKeyHandles(PM.L(layerId).p['scale.x'].kf[1]).hi;
    return { x: box.x + point[0], y: box.y + point[1] };
  }, id);
  await page.mouse.move(incoming.x, incoming.y);
  await page.mouse.down();
  await page.mouse.move(incoming.x + 10, incoming.y - 14, { steps: 8 });
  await page.mouse.up();

  await expect.poll(() => page.evaluate((layerId) => {
    const PM = (window as any).PM;
    const first = PM.UIState.getKeyHandles(PM.L(layerId).p['scale.x'].kf[0]);
    return first?.ho && first?.pt ? Math.hypot(first.ho[0] - first.pt[0], first.ho[1] - first.pt[1]) : 0;
  }, id)).toBeGreaterThan(4);
  expect(await page.evaluate((layerId) => (window as any).PM.L(layerId).p['scale.x'].kf.length, id)).toBe(2);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('continuous Bézier handles stay joined unless Option-drag splits them', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  const id = await scaleFixture(page);
  await page.evaluate((layerId) => {
    const PM = (window as any).PM;
    PM.setKey(PM.L(layerId), 'scale.x', 5, 140);
    PM.setKey(PM.L(layerId), 'scale.y', 5, 70);
    for (const key of ['scale.x', 'scale.y']) PM.L(layerId).p[key].kf[1].continuous = true;
    PM.touch(); PM.invalidate();
  }, id);
  await openGraph(page);
  await page.waitForFunction((layerId) => Boolean(
    (window as any).PM.UIState.getKeyHandles((window as any).PM.L(layerId).p['scale.x'].kf[1])?.ho
  ), id);
  const outgoing = await page.evaluate((layerId) => {
    const PM = (window as any).PM;
    const timeline = PM.Kernel.services.get('timeline');
    const box = timeline.cv.getBoundingClientRect();
    const point = PM.UIState.getKeyHandles(PM.L(layerId).p['scale.x'].kf[1]).ho;
    return { x: box.x + point[0], y: box.y + point[1] };
  }, id);
  const drag = async (option = false) => {
    if (option) await page.keyboard.down('Alt');
    await page.mouse.move(outgoing.x, outgoing.y);
    await page.mouse.down();
    await page.mouse.move(outgoing.x + 9, outgoing.y - 14, { steps: 8 });
    await page.mouse.up();
    if (option) await page.keyboard.up('Alt');
  };

  await drag();
  expect(await page.evaluate((layerId) => {
    const middle = (window as any).PM.L(layerId).p['scale.x'].kf[1];
    return { incoming: middle.ei, outgoing: middle.eo, mode: middle.bezierMode ?? null };
  }, id)).toEqual({
    incoming: expect.not.arrayContaining([1, 1]),
    outgoing: expect.not.arrayContaining([0, 0]),
    mode: 'continuous',
  });

  await page.keyboard.press('Meta+z');
  await drag(true);
  expect(await page.evaluate((layerId) => {
    const middle = (window as any).PM.L(layerId).p['scale.x'].kf[1];
    return { incoming: middle.ei, outgoing: middle.eo, mode: middle.bezierMode };
  }, id)).toEqual({
    incoming: [1, 1],
    outgoing: expect.not.arrayContaining([0, 0]),
    mode: 'split',
  });
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('marquee-selected graph keyframes move together from inside their transform box', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  const id = await scaleFixture(page);
  await openGraph(page);
  await page.waitForFunction((layerId) => {
    const PM = (window as any).PM, layer = PM.L(layerId);
    return ['scale.x', 'scale.y'].every(path => layer.p[path].kf.every((key: any) => PM.UIState.getKeyHandles(key)?.pt));
  }, id);
  const points = await page.evaluate((layerId) => {
    const PM = (window as any).PM;
    const timeline = PM.Kernel.services.get('timeline');
    const box = timeline.cv.getBoundingClientRect();
    const layer = PM.L(layerId);
    return ['scale.x', 'scale.y'].flatMap(path => layer.p[path].kf.map((key: any) => {
      const point = PM.UIState.getKeyHandles(key).pt;
      return { x: box.x + point[0], y: box.y + point[1] };
    }));
  }, id);
  const x0 = Math.min(...points.map(point => point.x)) - 10;
  const y0 = Math.min(...points.map(point => point.y)) - 10;
  const x1 = Math.max(...points.map(point => point.x)) + 10;
  const y1 = Math.max(...points.map(point => point.y)) + 10;
  await page.mouse.move(x0, y0);
  await page.mouse.down();
  await page.mouse.move(x1, y1, { steps: 8 });
  await page.mouse.up();
  expect(await page.evaluate(() => (window as any).PM.sel.keys.length)).toBe(4);

  const box = await page.evaluate(() => {
    const PM = (window as any).PM;
    const timeline = PM.Kernel.services.get('timeline');
    const canvas = timeline.cv.getBoundingClientRect();
    const bounds = timeline._graph.selectionBounds;
    return { x: canvas.x + (bounds.x0 + bounds.x1) / 2, y: canvas.y + (bounds.y0 + bounds.y1) / 2 };
  });
  const before = await page.evaluate((layerId) => {
    const layer = (window as any).PM.L(layerId);
    return ['scale.x', 'scale.y'].flatMap(path => layer.p[path].kf.map((key: any) => [key.t, key.v]));
  }, id);
  await page.mouse.move(box.x, box.y);
  await page.mouse.down();
  await page.mouse.move(box.x + 18, box.y - 12, { steps: 8 });
  await page.mouse.up();
  const after = await page.evaluate((layerId) => {
    const layer = (window as any).PM.L(layerId);
    return ['scale.x', 'scale.y'].flatMap(path => layer.p[path].kf.map((key: any) => [key.t, key.v]));
  }, id);
  expect(after).toHaveLength(before.length);
  expect(after.every((pair, index) => pair[0] > before[index]![0] && pair[1] > before[index]![1])).toBe(true);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('a Bézier handle adjusts paired Scale axes without changing other selected times', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  const id = await scaleFixture(page);
  await page.evaluate((layerId) => {
    const PM = (window as any).PM;
    const timeline = PM.Kernel.services.get('timeline');
    const layer = PM.L(layerId);
    PM.setKey(layer, 'scale.x', 5, 140);
    PM.setKey(layer, 'scale.y', 5, 70);
    PM.sel.keys = [
      layer.p['scale.x'].kf[0].i, layer.p['scale.y'].kf[0].i,
      layer.p['scale.x'].kf[1].i, layer.p['scale.y'].kf[1].i,
    ];
    timeline.keySelectionActive = true;
    PM.bus.emit('sel'); PM.touch(); PM.invalidate();
  }, id);
  await openGraph(page);
  await page.waitForFunction((layerId) => {
    const PM = (window as any).PM, layer = PM.L(layerId);
    return Boolean(PM.UIState.getKeyHandles(layer.p['scale.x'].kf[0])?.ho);
  }, id);
  const handle = await page.evaluate((layerId) => {
    const PM = (window as any).PM;
    const timeline = PM.Kernel.services.get('timeline');
    const box = timeline.cv.getBoundingClientRect();
    const point = PM.UIState.getKeyHandles(PM.L(layerId).p['scale.x'].kf[0]).ho;
    return { x: box.x + point[0], y: box.y + point[1] };
  }, id);

  await page.mouse.move(handle.x, handle.y);
  await page.mouse.down();
  await page.mouse.move(handle.x + 7, handle.y - 11, { steps: 8 });
  await page.mouse.up();

  expect(await page.evaluate((layerId) => {
    const layer = (window as any).PM.L(layerId);
    return ['scale.x', 'scale.y'].map(path => layer.p[path].kf.map((key: any) => key.eo));
  }, id)).toEqual([
    [expect.not.arrayContaining([0, 0]), [0, 0], [0, 0]],
    [expect.not.arrayContaining([0, 0]), [0, 0], [0, 0]],
  ]);
  await page.keyboard.press('Meta+z');
  expect(await page.evaluate((layerId) => {
    const layer = (window as any).PM.L(layerId);
    return ['scale.x', 'scale.y'].flatMap(path => layer.p[path].kf.map((key: any) => key.eo));
  }, id)).toEqual([[0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0]]);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('layer-strip clicks do not replace the curve focused in the Graph Editor', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await page.waitForFunction(() => { const PM = (window as any).PM; const timeline = PM.Kernel.services.get('timeline'); return Boolean(timeline?.cv); });
  const ids = await page.evaluate(() => {
    const PM = (window as any).PM;
    const timeline = PM.Kernel.services.get('timeline');
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: PM.mkProject({ name: 'Graph focus QA', dur: 10 }) })); PM.ProjectsScreen.hide();
    const focused = PM.mkLayer('solid', { name: 'Focused curve' });
    const other = PM.mkLayer('solid', { name: 'Other layer' });
    PM.proj.layers = [...PM.proj.layers, focused, other];
    PM.setKey(focused, 'opacity', 0, 0);
    PM.setKey(focused, 'opacity', 1, 100);
    PM.setKey(other, 'opacity', 0, 100);
    PM.setKey(other, 'opacity', 1, 0);
    PM.selectLayers(focused.id);
    PM.sel.chan = 'opacity';
    timeline.reveal(focused, ['opacity']);
    PM.invalidate();
    return { focused: focused.id, other: other.id };
  });
  await openGraph(page);
  await expect.poll(() => page.evaluate(() => { const PM = (window as any).PM; const timeline = PM.Kernel.services.get('timeline'); return timeline._graph?.target?.L?.name; })).toBe('Focused curve');
  const otherRow = await page.evaluate((otherId) => {
    const PM = (window as any).PM;
    const timeline = PM.Kernel.services.get('timeline');
    const box = timeline.cv.getBoundingClientRect();
    const index = timeline.rows.findIndex((row: any) => row.kind === 'layer' && row.L.id === otherId);
    return { x: box.x + 100, y: box.y + timeline.ruler + index * timeline.row - timeline.scrollY + timeline.row / 2 };
  }, ids.other);
  await page.mouse.click(otherRow.x, otherRow.y);
  expect(await page.evaluate((otherId) => (window as any).PM.sel.layers)).toEqual([ids.other]);
  expect(await page.evaluate(() => { const PM = (window as any).PM; const timeline = PM.Kernel.services.get('timeline'); return timeline.graphFocus; })).toEqual({ layerId: ids.focused, trackKey: 'opacity' });
  expect(await page.evaluate(() => (window as any).PM.sel.keys)).toEqual([]);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('project rename is available from the document menu and persists the live document', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await scaleFixture(page);
  const chip = page.locator('#doc-strip .project-doc');
  const id = await chip.getAttribute('data-project-id');
  await chip.click({ button: 'right' });
  await page.getByRole('menuitem', { name: 'Rename project…' }).click();
  const input = page.getByRole('textbox', { name: 'Rename project', exact: true });
  await input.fill('Renamed composition');
  await input.press('Enter');
  await expect(chip).toContainText('Renamed composition');
  await page.evaluate(() => (window as any).PM.flushProject());
  expect(await page.evaluate((id) => (window as any).PM.Projects.get(id).name, id)).toBe('Renamed composition');
  await session.relaunch();
  await session.openEditor();
  await expect(session.page.locator('#doc-strip .project-doc')).toContainText('Renamed composition');
});

test('empty timeline clicks preserve the playhead and marquee selection still works', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  const id = await scaleFixture(page);
  const key = await scaleKeyPoint(page, 3);
  const points = await page.evaluate(() => {
    const PM = (window as any).PM;
    const timeline = PM.Kernel.services.get('timeline');
    const box = timeline.cv.getBoundingClientRect();
    const x = box.x + timeline.gut + timeline.pps; // 1s, before the 2s clip start.
    return {
      layer: { x, y: box.y + timeline.ruler + timeline.row / 2 },
      prop: { x, y: box.y + timeline.ruler + timeline.row * 1.5 },
      below: { x, y: box.y + timeline.ruler + timeline.rows.length * timeline.row + 12 }
    };
  });
  for (const point of Object.values(points)) {
    await page.mouse.click(point.x, point.y);
    expect(await page.evaluate(() => (window as any).PM.time)).toBe(3);
  }
  await page.mouse.move(key.x - 12, key.y - 10);
  await page.mouse.down();
  await page.mouse.move(key.x + 12, key.y + 10, { steps: 5 });
  await page.mouse.up();
  expect(await page.evaluate(() => (window as any).PM.sel.keys.length)).toBe(2);
  expect(await page.evaluate(() => (window as any).PM.time)).toBe(3);
  await openGraph(page);
  await page.waitForFunction(() => { const PM = (window as any).PM; const timeline = PM.Kernel.services.get('timeline'); return Boolean(timeline._graph); });
  await page.mouse.click(points.prop.x, points.prop.y);
  expect(await page.evaluate(() => (window as any).PM.time)).toBe(3);
  expect(await page.evaluate((id) => (window as any).PM.L(id).from, id)).toBe(2);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('clicking the ruler moves the playhead', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await page.waitForFunction(() => { const PM = (window as any).PM; const timeline = PM.Kernel.services.get('timeline'); return Boolean(timeline?.cv); });
  await scaleFixture(page);
  await page.evaluate(() => new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
  const box = await page.locator('#tl-canvas').boundingBox();
  if (!box) throw new Error('no canvas');
  const gut = await page.evaluate(() => { const PM = (window as any).PM; const timeline = PM.Kernel.services.get('timeline'); return timeline.gut; });
  const ruler = await page.evaluate(() => { const PM = (window as any).PM; const timeline = PM.Kernel.services.get('timeline'); return timeline.ruler; });
  const before = await page.evaluate(() => (window as any).PM.time);
  // Click mid-ruler (below the thin work bar) well into the track area.
  await page.mouse.click(box.x + gut + 300, box.y + ruler - 6);
  const after = await page.evaluate(() => (window as any).PM.time);
  expect(after).not.toBe(before);
  expect(after).toBeGreaterThan(0);
  // Work area must be untouched by the scrub click.
  const work = await page.evaluate(() => (window as any).PM.proj.work);
  expect(work?.[0] ?? 0).toBe(0);
});

test('middle-button dragging pans the timeline viewport horizontally', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  const id = await scaleFixture(page);
  const before = await page.evaluate((id) => {
    const PM = (window as any).PM;
    const timeline = PM.Kernel.services.get('timeline');
    const box = timeline.cv.getBoundingClientRect();
    return {
      x: box.x + timeline.gut + 180,
      y: box.y + timeline.ruler + timeline.row / 2,
      scrollT: timeline.scrollT,
      time: PM.time,
      selected: [...PM.sel.layers],
      id
    };
  }, id);
  await page.mouse.move(before.x, before.y);
  await page.mouse.down({ button: 'middle' });
  await page.mouse.move(before.x - 90, before.y, { steps: 8 });
  await page.mouse.up({ button: 'middle' });

  const after = await page.evaluate(() => {
    const PM = (window as any).PM;
    const timeline = PM.Kernel.services.get('timeline');
    return { scrollT: timeline.scrollT, time: PM.time, selected: [...PM.sel.layers] };
  });
  expect(after.scrollT).toBeGreaterThan(before.scrollT + 1);
  expect(after.time).toBe(before.time);
  expect(after.selected).toEqual(before.selected);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('Shift pressed during a playhead drag snaps live to clip edges and keyframes', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await page.waitForFunction(() => { const PM = (window as any).PM; const timeline = PM.Kernel.services.get('timeline'); return Boolean(timeline?.cv); });
  await page.evaluate(() => {
    const PM = (window as any).PM;
    const timeline = PM.Kernel.services.get('timeline');
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: PM.mkProject({ name: 'Shift snap QA', dur: 8 }) })); PM.ProjectsScreen.hide();
    const layer = PM.mkLayer('solid', { name: 'Snap targets', from: 2, dur: 3 });
    PM.proj.layers = [...PM.proj.layers, layer];
    PM.setKey(layer, 'opacity', 3.4, 50);
    timeline.pps = 100;
    timeline.scrollT = 0;
    PM.invalidate('timeline');
  });
  await page.evaluate(() => new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
  const points = await page.evaluate(() => {
    const PM = (window as any).PM;
    const timeline = PM.Kernel.services.get('timeline');
    const box = timeline.cv.getBoundingClientRect();
    const point = (raw: number, target: number) => ({
      x: box.x + timeline.gut + raw * timeline.pps,
      y: box.y + timeline.ruler - 6,
      target,
    });
    return [point(1.93, 2), point(3.33, 3.4), point(4.93, 5)];
  });

  for (const point of points) {
    await page.mouse.move(point.x, point.y);
    await page.mouse.down();
    const raw = await page.evaluate(() => (window as any).PM.time);
    expect(Math.abs(raw - point.target)).toBeGreaterThan(0.02);

    await page.keyboard.down('Shift');
    await expect.poll(() => page.evaluate(() => (window as any).PM.time)).toBeCloseTo(point.target, 6);

    await page.keyboard.up('Shift');
    await expect.poll(() => page.evaluate(() => (window as any).PM.time)).toBeCloseTo(raw, 6);
    await page.mouse.up();
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('clicking the transport button pauses playback', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await scaleFixture(page);
  const transport = page.getByRole('button', { name: 'Play / Pause (Space)' });

  await transport.click();
  await expect.poll(() => page.evaluate(() => (window as any).PM.playing)).toBe(true);
  await expect.poll(() => page.evaluate(() => (window as any).PM.time)).toBeGreaterThan(0);

  const box = await transport.boundingBox();
  if (!box) throw new Error('no transport button');
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.waitForTimeout(100);
  await page.mouse.up();
  await expect.poll(() => page.evaluate(() => (window as any).PM.playing)).toBe(false);
  const pausedAt = await page.evaluate(() => (window as any).PM.time);
  await page.waitForTimeout(150);
  expect(await page.evaluate(() => (window as any).PM.time)).toBe(pausedAt);
});
