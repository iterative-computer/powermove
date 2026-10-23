import type { Page } from '@playwright/test';
import { expect, test } from './helpers/app';

async function newProject(page: Page, name: string) {
  await page.waitForFunction(() => !(window as any).PM.projectLoading);
  await page.evaluate(() => (window as any).PM.cmd('newProject'));
  const modal = page.locator('.modal');
  await modal.getByLabel('Project name').fill(name);
  await modal.getByLabel('Width in pixels').fill('320');
  await modal.getByLabel('Height in pixels').fill('180');
  await modal.getByLabel('Duration in seconds').fill('2');
  await modal.getByRole('button', { name: 'Create', exact: true }).click();
  await page.waitForFunction(name => {
    const PM = (window as any).PM;
    return PM.proj.name === name && !PM.projectLoading && PM.GL?.gl;
  }, name);
}

test('Ungroup keeps nested hidden members invisible and restores their visibility animation with Undo', async ({ session }) => {
  await session.openEditor(); await newProject(session.page, 'Ungroup hidden groups');
  const result = await session.page.evaluate(() => {
    const PM = (window as any).PM; PM.pause(); PM.time = .5;
    const first = PM.mkLayer('shape', { d: { shape: 'rect', w: 70, h: 60, color: '#ffffff', radius: 0, stroke: 0 }, p: { 'position.x': 100, 'position.y': 90 } });
    const second = PM.cloneLayer(first);
    first.on = PM.P(true, { kf: [PM.KF(0, true, 'hold'), PM.KF(1, false, 'hold')] });
    second.on = PM.P(true, { expr: 'time < 2' });
    const outside = PM.mkLayer('shape', { d: { shape: 'rect', w: 40, h: 40, color: '#40a0ff', radius: 0, stroke: 0 }, p: { 'position.x': 260, 'position.y': 90 } });
    PM.proj.layers = [first, second, outside]; PM.ProjectIndex.invalidate(); PM.touch();
    const inner = PM.groupLayers([first.id, second.id]), outer = PM.groupLayers([inner.id]);
    inner.p.opacity.v = 50; outer.p.opacity.v = 80; PM.touch();
    const render = (time: number) => PM.GL.renderToPixels(time, 320, 180, { transparent: true, mblur: false });
    const visibleAlpha = render(.5)[(90 * 320 + 100) * 4 + 3];
    PM.selectLayers(inner.id);
    const hide = PM.Edit.apply({ type: 'set_layer', target: inner.id, patch: { visible: false } }, { origin: 'timeline', label: 'Hide group' });
    if (!hide.ok) throw new Error(hide.message);
    outer.on = PM.P(true, { kf: [PM.KF(0, false, 'hold'), PM.KF(2, false, 'hold')] });
    PM.selectLayers([outer.id, inner.id]); PM.touch(); PM.hist.clear();
    const originalOn = [first, second].map(layer => JSON.stringify(layer.on));
    const times = [.5, 1.5], before = times.map(render);
    const difference = () => times.map((time, frame) => {
      const pixels = render(time); let max = 0;
      for (let index = 0; index < pixels.length; index++) max = Math.max(max, Math.abs(pixels[index] - before[frame][index]));
      return max;
    });
    const edit = PM.cmd('ungroupLayers'); if (!edit.ok) throw new Error(edit.message);
    const after = difference(), hidden = [first, second].every(layer => PM.L(layer.id).on === false);
    const removed = !PM.L(inner.id) && !PM.L(outer.id), undo = PM.hist.undo(), restored = difference();
    const tracksRestored = [first, second].every((layer, index) => JSON.stringify(PM.L(layer.id).on) === originalOn[index]);
    const redo = PM.hist.redo(), redone = difference();
    return { visibleAlpha, centerAlpha: before.map(pixels => pixels[(90 * 320 + 100) * 4 + 3]),
      outsideAlpha: before.map(pixels => pixels[(90 * 320 + 260) * 4 + 3]), after, hidden, removed,
      undo, restored, tracksRestored, redo, redone, history: PM.hist.list() };
  });
  expect(result.visibleAlpha).toBeGreaterThan(90);
  expect(result.centerAlpha).toEqual([0, 0]); expect(result.outsideAlpha).toEqual([255, 255]);
  expect(result.hidden).toBe(true); expect(result.removed).toBe(true); expect(result.undo).toBe(true);
  expect(result.tracksRestored).toBe(true); expect(result.redo).toBe(true); expect(result.history).toEqual(['Ungroup layers']);
  [...result.after, ...result.restored, ...result.redone].forEach(delta => expect(delta).toBe(0));
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('Ungroup shows a visible error and preserves an animated visibility boundary', async ({ session }) => {
  await session.openEditor(); await newProject(session.page, 'Ungroup animated visibility');
  const result = await session.page.evaluate(() => {
    const PM = (window as any).PM; PM.pause(); PM.time = .5;
    const child = PM.mkLayer('shape', { d: { shape: 'rect', w: 70, h: 60, color: '#ffffff', radius: 0, stroke: 0 }, p: { 'position.x': 160, 'position.y': 90 } });
    PM.proj.layers = [child]; PM.ProjectIndex.invalidate(); PM.touch();
    const group = PM.groupLayers([child.id]); group.on = PM.P(true, { expr: 'time >= 1' }); PM.touch(); PM.hist.clear();
    const times = [.5, 1.5], before = times.map(time => PM.GL.renderToPixels(time, 320, 180, { transparent: true, mblur: false }));
    const source = PM.serialize(), selection = JSON.stringify(PM.sel);
    const edit = PM.cmd('ungroupLayers');
    const differences = times.map((time, frame) => {
      const pixels = PM.GL.renderToPixels(time, 320, 180, { transparent: true, mblur: false }); let max = 0;
      for (let index = 0; index < pixels.length; index++) max = Math.max(max, Math.abs(pixels[index] - before[frame][index]));
      return max;
    });
    return { ok: edit.ok, message: edit.message, centerAlpha: before.map(pixels => pixels[(90 * 320 + 160) * 4 + 3]),
      differences, unchanged: PM.serialize() === source, selected: JSON.stringify(PM.sel) === selection, history: PM.hist.list(), undo: PM.hist.undo() };
  });
  await expect(session.page.locator('.toast').filter({ hasText: 'Pre-compose this group to keep its animated visibility' })).toBeVisible();
  expect(result.ok).toBe(false); expect(result.message).toContain('Pre-compose this group to keep its animated visibility');
  expect(result.centerAlpha).toEqual([0, 255]); expect(result.differences).toEqual([0, 0]);
  expect(result.unchanged).toBe(true); expect(result.selected).toBe(true); expect(result.history).toEqual([]); expect(result.undo).toBe(false);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
