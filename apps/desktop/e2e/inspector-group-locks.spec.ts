import { chooseNativeMenu, expect, inspectNativeMenu, test } from './helpers/app';

test('inspector edits skip selected layers inside locked groups and Undo restores editable siblings', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  const fixture = await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.replaceProject(PM.mkProject({ name: 'Inspector group locks', dur: 5 }));
    const editable = PM.mkLayer('shape', { name: 'Editable sibling' });
    const protectedLayer = PM.mkLayer('shape', { name: 'Protected child' });
    PM.proj.layers = [editable, protectedLayer]; PM.ProjectIndex.invalidate();
    for (const layer of [editable, protectedLayer]) {
      PM.setKey(layer, 'opacity', 0, 20);
      PM.setKey(layer, 'opacity', 1, 80);
    }
    const inner = PM.groupLayers([protectedLayer.id], 'Inner group');
    const outer = PM.groupLayers([inner.id], 'Locked outer group');
    outer.lock = true;
    PM.selectLayers([editable.id, protectedLayer.id]);
    PM.setTime(1); PM.hist.clear(); PM.touch(); PM.bus.emit('layers'); PM.invalidate();
    return { editable: editable.id, protected: protectedLayer.id, keys: JSON.stringify(protectedLayer.p.opacity) };
  });
  const state = () => page.evaluate(fixture => {
    const PM = (window as any).PM;
    const editable = PM.L(fixture.editable);
    return { count: editable.p.opacity.kf.length, value: PM.ev(editable, 'opacity', PM.time),
      protected: JSON.stringify(PM.L(fixture.protected).p.opacity) };
  }, fixture);

  await page.getByRole('button', { name: 'Remove keyframe for Opacity', exact: true }).click();
  await expect.poll(state).toEqual({ count: 1, value: 20, protected: fixture.keys });
  await page.keyboard.press('Meta+z');
  await expect.poll(state).toEqual({ count: 2, value: 80, protected: fixture.keys });

  await page.evaluate(() => (window as any).PM.setTime(2));
  await page.getByRole('button', { name: 'Add keyframe for Opacity', exact: true }).click();
  await expect.poll(state).toEqual({ count: 3, value: 80, protected: fixture.keys });
  await page.keyboard.press('Meta+z');
  await expect.poll(state).toEqual({ count: 2, value: 80, protected: fixture.keys });

  const opacity = page.getByRole('spinbutton', { name: 'Opacity', exact: true });
  await opacity.click();
  await opacity.fill('60'); await opacity.press('Enter');
  await expect.poll(state).toEqual({ count: 3, value: 60, protected: fixture.keys });
  await page.keyboard.press('Meta+z');
  await expect.poll(state).toEqual({ count: 2, value: 80, protected: fixture.keys });
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('inspector context menus protect nested group locks and allow normal edits after unlocking', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  const fixture = await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.replaceProject(PM.mkProject({ name: 'Inspector menu locks', dur: 5 }));
    const layer = PM.mkLayer('shape', { name: 'Protected menu target' });
    PM.proj.layers = [layer]; PM.ProjectIndex.invalidate();
    PM.setKey(layer, 'opacity', 0, 20); PM.setKey(layer, 'opacity', 2, 80);
    const inner = PM.groupLayers([layer.id], 'Inner');
    const outer = PM.groupLayers([inner.id], 'Locked outer');
    outer.lock = true;
    PM.selectLayers(layer.id); PM.setTime(1); PM.hist.clear(); PM.touch(); PM.bus.emit('layers'); PM.invalidate();
    return { layer: layer.id, group: outer.id, keys: JSON.stringify(layer.p.opacity) };
  });
  const row = page.locator('[data-channel="opacity"]');
  await expect(row).toBeVisible();
  const menu = await inspectNativeMenu(session, () => row.click({ button: 'right' }));
  for (const label of ['Add Keyframe at Playhead', 'Remove Animation', 'Power', 'Add Expression…', 'Reset']) {
    expect(menu.find(item => item.label === label), label).toMatchObject({ enabled: false });
  }
  expect(menu.find(item => item.label === 'Show in Graph Editor')).toMatchObject({ enabled: true });
  const source = () => page.evaluate(id => JSON.stringify((window as any).PM.L(id).p.opacity), fixture.layer);
  expect(await source()).toBe(fixture.keys);

  expect(await page.evaluate(id => (window as any).PM.Edit.apply({ type: 'set_layer', target: id,
    patch: { locked: false } }, { origin: 'inspector' }).ok, fixture.group)).toBe(true);
  await chooseNativeMenu(session, 'Add Keyframe at Playhead', () => row.click({ button: 'right' }));
  await expect.poll(() => page.evaluate(id => (window as any).PM.L(id).p.opacity.kf.length, fixture.layer)).toBe(3);
  expect(await page.evaluate(() => (window as any).PM.hist.undo())).toBe(true);
  await expect.poll(source).toBe(fixture.keys);
  await chooseNativeMenu(session, 'Power', () => row.click({ button: 'right' }));
  await expect.poll(source).not.toBe(fixture.keys);
  expect(await page.evaluate(() => (window as any).PM.hist.undo())).toBe(true);
  await expect.poll(source).toBe(fixture.keys);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
