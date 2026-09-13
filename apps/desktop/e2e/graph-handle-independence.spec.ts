import { expect, test } from './helpers/app';

test.beforeEach(async ({ session }) => { await session.openEditor(); });

test('dragging an incoming handle leaves another selected key handle fixed and supports Undo', async ({ session }) => {
  const { page } = session;
  await page.waitForFunction(() => { const PM = (window as any).PM, timeline = PM.Kernel.services.get('timeline'); return Boolean(timeline?.cv); });
  await page.evaluate(() => {
    const PM = (window as any).PM;
    const timeline = PM.Kernel.services.get('timeline');
    PM.replaceProject(PM.mkProject({ name: 'Independent handles', dur: 6 }));
    const L = PM.mkLayer('text', { dur: 6 });
    PM.proj.layers.push(L);
    const prop = PM.findProp(L, 'position.x');
    for (const [t, v] of [[0, 500], [3, 100], [5, 250]]) PM.setKeyOn(prop, t, v);
    for (const key of prop.kf) { key.ei = [0.3, 0.7]; key.eo = [0.6, 0]; key.bezierMode = 'split'; }
    PM.selectLayers(L.id); PM.sel.chan = 'position.x';
    PM.sel.keys = prop.kf.map((k: any) => k.i);
    timeline.pps = 90; timeline.scrollT = 0;
    PM.bus.emit('layers'); PM.invalidate();
  });
  await page.getByRole('button', { name: 'Graph editor (Shift+F3)', exact: true }).click();
  await page.waitForFunction(() => {
    const PM = (window as any).PM;
    const timeline = PM.Kernel.services.get('timeline');
    return timeline._graph?.points.length === 3;
  });
  const before = await page.evaluate(() => {
    const PM = (window as any).PM, prop = PM.findProp(PM.proj.layers[0], 'position.x');
    const timeline = PM.Kernel.services.get('timeline');
    const box = timeline.cv.getBoundingClientRect(), p = PM.UIState.getKeyHandles(prop.kf[2]).hi;
    PM.hist.clear();
    return { source: JSON.stringify(prop), other: JSON.stringify(prop.kf[1]), handle: [...prop.kf[2].ei], x: box.x + p[0], y: box.y + p[1] };
  });
  await page.mouse.move(before.x, before.y); await page.mouse.down();
  await page.mouse.move(before.x + 16, before.y - 18, { steps: 6 }); await page.mouse.up();
  const after = await page.evaluate(() => {
    const PM = (window as any).PM, prop = PM.findProp(PM.proj.layers[0], 'position.x');
    return { other: JSON.stringify(prop.kf[1]), handle: [...prop.kf[2].ei] };
  });
  expect(after.handle).not.toEqual(before.handle);
  expect(after.other).toBe(before.other);
  await page.evaluate(() => (window as any).PM.hist.undo());
  expect(await page.evaluate(() => {
    const PM = (window as any).PM;
    return JSON.stringify(PM.findProp(PM.proj.layers[0], 'position.x'));
  })).toBe(before.source);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
