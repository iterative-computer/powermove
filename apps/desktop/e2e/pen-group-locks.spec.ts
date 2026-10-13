import { expect, test, chooseNativeMenu } from './helpers/app';

test('Pen edits protect nested locked groups and work after unlocking with one Undo', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await page.waitForFunction(() => Boolean((window as any).PM?.GL?.gl));
  const fixture = await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.replaceProject(PM.mkProject({ w: 480, h: 270, dur: 3 }));
    const values = { x: 0, y: 0, rotation: 0, scaleX: 100, scaleY: 100, closed: false,
      fill: '#E8E2CF', fillEnabled: true, fillOpacity: 100, stroke: '#ffffff', strokeWidth: 2,
      strokeOpacity: 100, trimStart: 0, trimEnd: 100, trimOffset: 0, copies: 1,
      repeatX: 30, repeatY: 0, repeatRotation: 0 };
    const path = { id: 'pen-path', name: 'Path', parent: null,
      p: Object.fromEntries(Object.entries(values).map(([key, value]) => [key, PM.P(value)])),
      vertices: [[-50, -50], [50, -50], [0, 50]].map(([x, y], index) => ({ id: `vertex-${index}`,
        p: Object.fromEntries(Object.entries({ x, y, inX: 0, inY: 0, outX: 0, outY: 0 }).map(([key, value]) => [key, PM.P(value)])) })) };
    const added = PM.Edit.apply({ type: 'add_layer', id: 'pen-shape', layerType: 'shape',
      content: { paths: [path] }, properties: { 'position.x': 240, 'position.y': 135 }, select: true });
    if (!added.ok) throw new Error(added.message);
    const inner = PM.groupLayers(['pen-shape']), outer = PM.groupLayers([inner.id]);
    outer.lock = true; PM.selectLayers('pen-shape'); PM.hist.clear(); PM.touch(); PM.invalidate();
    const viewer = PM.Kernel.services.get('viewer');
    viewer.fit = true; viewer.activePath = path.id; viewer.layout();
    return { group: outer.id, paths: JSON.stringify(PM.L('pen-shape').d.paths) };
  });
  await chooseNativeMenu(session, 'Pen Tool (G)', () => page.getByRole('button', { name: 'Drawing tools', exact: true }).click());
  const point = async (x: number, y: number) => {
    const bounds = (await page.locator('#stage-inner').boundingBox())!;
    const scale = await page.evaluate(() => (window as any).PM.Kernel.services.get('viewer').shown);
    return { x: bounds.x + x * scale, y: bounds.y + y * scale };
  };
  const drag = async () => {
    const start = await point(290, 85), end = await point(310, 105);
    await page.mouse.move(start.x, start.y); await page.mouse.down();
    await page.mouse.move(end.x, end.y, { steps: 4 }); await page.mouse.up();
  };
  await drag();
  expect(await page.evaluate(() => JSON.stringify((window as any).PM.L('pen-shape').d.paths))).toBe(fixture.paths);
  expect(await page.evaluate(() => (window as any).PM.hist.list().length)).toBe(0);
  const lockedPoint = await point(290, 85);
  await page.mouse.dblclick(lockedPoint.x, lockedPoint.y);
  expect(await page.evaluate(() => JSON.stringify((window as any).PM.L('pen-shape').d.paths))).toBe(fixture.paths);
  expect(await page.evaluate(() => (window as any).PM.hist.list().length)).toBe(0);
  await page.evaluate(group => {
    const PM = (window as any).PM; PM.L(group).lock = false; PM.touch(); PM.invalidate();
  }, fixture.group);
  await drag();
  expect(await page.evaluate(() => JSON.stringify((window as any).PM.L('pen-shape').d.paths))).not.toBe(fixture.paths);
  expect(await page.evaluate(() => (window as any).PM.hist.undo())).toBe(true);
  expect(await page.evaluate(() => JSON.stringify((window as any).PM.L('pen-shape').d.paths))).toBe(fixture.paths);
  const history = await page.evaluate(() => (window as any).PM.hist.list().length);
  const start = await point(290, 85), end = await point(310, 105);
  await page.mouse.move(start.x, start.y); await page.mouse.down();
  await page.mouse.move(end.x, end.y, { steps: 4 });
  await expect.poll(() => page.evaluate(() => JSON.stringify((window as any).PM.L('pen-shape').d.paths))).not.toBe(fixture.paths);
  await page.keyboard.press('Escape'); await page.mouse.up();
  expect(await page.evaluate(() => JSON.stringify((window as any).PM.L('pen-shape').d.paths))).toBe(fixture.paths);
  expect(await page.evaluate(() => (window as any).PM.hist.list().length)).toBe(history);
  expect(await page.evaluate(() => (window as any).PM.sel.layers)).toEqual(['pen-shape']);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
