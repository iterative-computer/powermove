import { expect, test } from './helpers/app';

/* Every light layer shows in the viewer: a marker you can see, click to
   select, and drag to move (one Undo, keyed when Position is animated). */

type Marker = { id: string; name: string; type: string; x: number; y: number; edge: number | null; behind: boolean; selected: boolean; hasCone: boolean; hasRange: boolean };

test('light markers show, select, drag, key and pin to the edge in both views', async ({ session }) => {
  test.setTimeout(90_000);
  await session.openEditor();
  const page = session.page;
  const ids = await page.evaluate(() => {
    const PM = (window as any).PM, api = PM.Kernel.api('light-markers-e2e');
    api.scene3d.edit({ operation: 'add_object', object: { source: { primitive: 'box' }, name: 'Box' } });
    const point = api.scene3d.edit({ operation: 'add_light', light: { type: 'point', name: 'Point light', p: { x: 1.6, y: 1.2, z: 1.5, color: '#FFB46B', intensity: 20, distance: 4 } } });
    const spot = api.scene3d.edit({ operation: 'add_light', light: { type: 'spot', name: 'Spot light', p: { x: -2.4, y: 1.3, z: 1.2, angle: 22 } } });
    if (!point.ok || !spot.ok) throw new Error(point.message || spot.message);
    PM.selectLayers([]);
    const find = (name: string) => PM.proj.layers.find((layer: any) => layer.name === name).id;
    return { point: find('Point light'), spot: find('Spot light') };
  });
  const stage = page.locator('#stage').first();
  const markers = () => page.evaluate(() => (window as any).PM.Kernel.services.get('viewer').lightMarkers() as Marker[]);
  const markerFor = async (id: string) => (await markers()).find(marker => marker.id === id)!;
  const client = async (marker: Marker) => { const box = (await stage.boundingBox())!; return { x: box.x + marker.x, y: box.y + marker.y }; };
  const position = (id: string) => page.evaluate(layerId => {
    const layer = (window as any).PM.L(layerId); return ['x', 'y', 'z'].map(axis => layer.p[`position.${axis}`].v as number);
  }, id);

  // 1. Unselected lights are visible without touching the inspector.
  await expect.poll(async () => (await markers()).length).toBe(2);
  const point = await markerFor(ids.point), spot = await markerFor(ids.spot);
  expect(point).toMatchObject({ type: 'point', edge: null, selected: false });
  expect(spot).toMatchObject({ type: 'spot', edge: null, hasCone: true });

  // 2. Clicking the marker selects that light layer.
  const pointAt = await client(point);
  await page.mouse.click(pointAt.x, pointAt.y);
  await expect.poll(() => page.evaluate(() => (window as any).PM.sel.layers)).toEqual([ids.point]);
  expect((await markerFor(ids.point))).toMatchObject({ selected: true, hasRange: true });

  // 3. Dragging moves it on the camera plane, even in Rotate mode, as one Undo step.
  await page.evaluate(() => (window as any).PM.Kernel.api('light-markers-e2e').scene3d.setMode('rotate'));
  const before = await position(ids.point);
  const renderBefore = await page.evaluate(() => Array.from((window as any).PM.GL.renderToPixels(0, 96, 54, { mblur: false }) as Uint8Array).join(','));
  await expect(async () => {
    const from = await client(await markerFor(ids.point));
    await page.mouse.move(from.x, from.y); await page.mouse.down();
    for (let i = 1; i <= 8; i++) await page.mouse.move(from.x - i * 25, from.y + i * 10);
    await page.mouse.up();
    const moved = await position(ids.point);
    expect(moved[0]).toBeLessThan(before[0]! - .5);
    expect(moved[1]).toBeLessThan(before[1]!);
    expect(moved[2]).toBeCloseTo(before[2]!, 5);
  }).toPass({ timeout: 15_000 });
  const renderAfter = await page.evaluate(() => Array.from((window as any).PM.GL.renderToPixels(0, 96, 54, { mblur: false }) as Uint8Array).join(','));
  expect(renderAfter).not.toBe(renderBefore);
  await page.evaluate(() => (window as any).PM.hist.undo());
  expect(await position(ids.point)).toEqual(before);

  // 4. With Position animated, the drag keys the current time instead of overwriting.
  await page.evaluate(({ id }) => {
    const PM = (window as any).PM;
    for (const axis of ['x', 'y', 'z']) PM.Edit.apply({ type: 'set_property', target: id, path: `position.${axis}`, value: PM.L(id).p[`position.${axis}`].v, time: 0, mode: 'keyframe' });
    PM.Kernel.api('light-markers-e2e').transport.setTime(1);
  }, { id: ids.point });
  await page.waitForTimeout(200);
  await expect(async () => {
    const from = await client(await markerFor(ids.point));
    await page.mouse.move(from.x, from.y); await page.mouse.down();
    for (let i = 1; i <= 6; i++) await page.mouse.move(from.x + i * 20, from.y);
    await page.mouse.up();
    const keys = await page.evaluate(id => (window as any).PM.L(id).p['position.x'].kf.map((k: any) => k.t), ids.point);
    expect(keys.length).toBe(2);
  }).toPass({ timeout: 15_000 });

  // 5. Off-screen and behind-camera lights pin to the stage edge and still select.
  await page.evaluate(({ point, spot }) => {
    const PM = (window as any).PM, api = PM.Kernel.api('light-markers-e2e');
    // The drag marked Position as hand-edited; overwrite it explicitly (keys the current time).
    const moves = Object.entries({ x: 9, y: .5, z: 1 }).map(([axis, value]) => ({ type: 'set_property', target: point, path: `position.${axis}`, value, preserveHandEdits: false }));
    const result = PM.Edit.apply(moves);
    if (!result.ok) throw new Error(result.message);
    const spotMove = api.scene3d.edit({ operation: 'update_light', target: spot, patch: { p: { z: 9 } } });
    if (!spotMove.ok) throw new Error(spotMove.message);
    PM.selectLayers([]);
  }, ids);
  await expect.poll(async () => (await markerFor(ids.point)).edge).not.toBeNull();
  expect(await markerFor(ids.spot)).toMatchObject({ behind: true });
  const pinned = await client(await markerFor(ids.spot));
  await page.mouse.click(pinned.x, pinned.y);
  await expect.poll(() => page.evaluate(() => (window as any).PM.sel.layers)).toEqual([ids.spot]);

  // 6. Free View projects markers through the inspection camera.
  await page.evaluate(({ point, spot }) => {
    const PM = (window as any).PM, api = PM.Kernel.api('light-markers-e2e');
    PM.Edit.apply(Object.entries({ x: 1.6, y: 1.2, z: 1.5 }).map(([axis, value]) => ({ type: 'set_property', target: point, path: `position.${axis}`, value, preserveHandEdits: false })));
    api.scene3d.edit({ operation: 'update_light', target: spot, patch: { p: { z: 1.2 } } });
  }, ids);
  const cameraView = await markerFor(ids.point);
  await page.evaluate(() => { const api = (window as any).PM.Kernel.api('light-markers-e2e'); api.scene3d.setView('editor'); api.scene3d.frame(false); });
  await expect.poll(async () => {
    const free = await markerFor(ids.point);
    return Math.hypot(free.x - cameraView.x, free.y - cameraView.y) > 5;
  }).toBe(true);
  await page.evaluate(() => (window as any).PM.Kernel.api('light-markers-e2e').scene3d.setView('camera'));

  // 7. Layer Controls (⌘⇧H) hides markers; renders never contain them.
  const pixels = () => page.evaluate(() => Array.from((window as any).PM.GL.renderToPixels(0, 96, 54, { mblur: false }) as Uint8Array).join(','));
  const shown = await pixels();
  await page.evaluate(() => (window as any).PM.cmd('toggleLayerControls'));
  await expect.poll(async () => (await markers()).length).toBe(0);
  expect(await pixels()).toBe(shown);
  await page.evaluate(() => (window as any).PM.cmd('toggleLayerControls'));
  await expect.poll(async () => (await markers()).length).toBe(2);
});
