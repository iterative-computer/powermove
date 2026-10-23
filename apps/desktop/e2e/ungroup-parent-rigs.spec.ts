import path from 'node:path';
import { expect, test } from './helpers/app';

test('Ungroup preserves an outside animated child, Undo and saved project pixels', async ({ session }) => {
  test.setTimeout(90_000);
  await session.openEditor();
  await session.page.waitForFunction(() => !(window as any).PM.projectLoading);
  await session.page.evaluate(() => (window as any).PM.cmd('newProject'));
  const modal = session.page.locator('.modal');
  await modal.getByLabel('Project name').fill('Ungroup parent regression');
  await modal.getByLabel('Width in pixels').fill('320');
  await modal.getByLabel('Height in pixels').fill('180');
  await modal.getByLabel('Duration in seconds').fill('2');
  await modal.getByRole('button', { name: 'Create', exact: true }).click();
  await session.page.waitForFunction(() => {
    const PM = (window as any).PM;
    return PM.proj.name === 'Ungroup parent regression' && !PM.projectLoading && PM.GL?.gl;
  });

  const fixture = await session.page.evaluate(() => {
    const PM = (window as any).PM; PM.pause();
    const project = PM.proj;
    const first = PM.mkLayer('shape', { name: 'First member', d: { shape: 'rect', w: 40, h: 40, color: '#ffffff', stroke: 0 }, p: { 'position.x': 60, 'position.y': 90 } }, project);
    const second = PM.mkLayer('shape', { name: 'Second member', d: { shape: 'rect', w: 40, h: 40, color: '#40a0ff', stroke: 0 }, p: { 'position.x': 100, 'position.y': 90 } }, project);
    const outside = PM.mkLayer('shape', { name: 'Outside dependent', d: { shape: 'rect', w: 50, h: 30, color: '#e040c0', stroke: 0 }, p: { 'position.x': 230, 'position.y': 55 } }, project);
    project.layers = [first, second, outside]; PM.ProjectIndex.invalidate(); PM.touch();
    const grouped = PM.Edit.apply({ type: 'group_layers', targets: [first.id, second.id], name: 'Animated parent' }, { origin: 'agent' });
    if (!grouped.ok) throw new Error(grouped.message);
    const group = PM.firstSel();
    const parented = PM.Edit.apply({ type: 'set_layer', target: outside.id, patch: { parent: group.id } }, { origin: 'agent' });
    if (!parented.ok) throw new Error(parented.message);
    group.p.rotation.kf = [PM.KF(0, 0, 'linear'), PM.KF(2, 40, 'linear')];
    const locked = PM.Edit.apply({ type: 'set_layer', target: outside.id, patch: { locked: true } }, { origin: 'inspector' });
    if (!locked.ok) throw new Error(locked.message);
    PM.touch(); PM.hist.clear();
    const times = [.5, 1.5];
    const render = (time: number) => PM.GL.renderToPixels(time, 320, 180, { transparent: true, mblur: false });
    const reference = times.map(render);
    const coloredPixels = reference.map((pixels: Uint8Array) => {
      let count = 0;
      for (let index = 0; index < pixels.length; index += 4) {
        if (pixels[index] > 150 && pixels[index + 1] < 110 && pixels[index + 2] > 140 && pixels[index + 3] > 200) count++;
      }
      return count;
    });
    const difference = () => times.map((time, frame) => {
      const pixels = render(time); let max = 0;
      for (let index = 0; index < pixels.length; index++) max = Math.max(max, Math.abs(pixels[index] - reference[frame][index]));
      return max;
    });
    const edited = PM.Edit.apply({ type: 'ungroup_layers', targets: [group.id] }, { origin: 'agent', label: 'Ungroup layers' });
    if (!edited.ok) throw new Error(edited.message);
    const after = difference(), rig = PM.L(group.id).type;
    const undo = PM.hist.undo(), restored = difference(), undoneType = PM.L(group.id).type;
    const redo = PM.hist.redo(), redone = difference();
    return { projectId: PM.proj.id, groupId: group.id, outsideId: outside.id, times,
      reference: reference.map((pixels: Uint8Array) => Array.from(pixels)), coloredPixels,
      rig, undo, restored, undoneType, redo, redone, after, history: PM.hist.list() };
  });
  fixture.coloredPixels.forEach(count => expect(count).toBeGreaterThan(500));
  expect(fixture.rig).toBe('null'); expect(fixture.undoneType).toBe('group');
  expect(fixture.undo).toBe(true); expect(fixture.redo).toBe(true);
  expect(fixture.history).toEqual(['Ungroup layers']);
  [...fixture.after, ...fixture.restored, ...fixture.redone].forEach(delta => expect(delta).toBeLessThanOrEqual(2));

  await session.app.evaluate(({ dialog }, filePath) => {
    dialog.showSaveDialog = async () => ({ canceled: false, filePath });
  }, path.join(session.userData, 'ungroup-parent-rig.pmv'));
  expect(await session.page.evaluate(() => (window as any).PM.saveProject())).toBe(true);
  await session.page.evaluate(() => (window as any).PM.flushProject());
  await session.relaunch();
  await session.openEditor();
  await session.page.waitForFunction((id) => {
    const PM = (window as any).PM;
    return PM.proj.id === id && !PM.projectLoading && Boolean(PM.GL?.gl);
  }, fixture.projectId);
  const loaded = await session.page.evaluate(({ groupId, outsideId, times, reference }) => {
    const PM = (window as any).PM; PM.pause();
    const difference = times.map((time, frame) => {
      const pixels = PM.GL.renderToPixels(time, 320, 180, { transparent: true, mblur: false }); let max = 0;
      for (let index = 0; index < pixels.length; index++) max = Math.max(max, Math.abs(pixels[index] - reference[frame][index]));
      return max;
    });
    return { type: PM.L(groupId).type, parent: PM.L(outsideId).parent, locked: PM.L(outsideId).lock, difference };
  }, { groupId: fixture.groupId, outsideId: fixture.outsideId, times: fixture.times, reference: fixture.reference });
  expect(loaded.type).toBe('null'); expect(loaded.parent).toBe(fixture.groupId); expect(loaded.locked).toBe(true);
  loaded.difference.forEach(delta => expect(delta).toBeLessThanOrEqual(2));
  expect(session.diagnostics.pageErrors).toEqual([]);
});
