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

for (const unsafe of ['overlapping children', 'overrange child'] as const) {
  test(`Ungroup preserves actual pixels when rejecting ${unsafe}`, async ({ session }) => {
    await session.openEditor();
    await newProject(session.page, `Ungroup ${unsafe}`);
    const result = await session.page.evaluate(unsafe => {
      const PM = (window as any).PM; PM.pause(); PM.time = 1;
      const first = PM.mkLayer('shape', {
        d: { shape: 'rect', w: 80, h: 60, color: '#ffffff', radius: 0, stroke: 0 },
        p: { 'position.x': 160, 'position.y': 90 },
      });
      const layers = unsafe === 'overlapping children' ? [first, PM.cloneLayer(first)] : [first];
      PM.proj.layers = layers; PM.ProjectIndex.invalidate(); PM.touch();
      const grouped = PM.Edit.apply({ type: 'group_layers', targets: layers.map(layer => layer.id) }, { origin: 'agent' });
      if (!grouped.ok) throw new Error(grouped.message);
      const group = PM.firstSel(); group.p.opacity.v = 50;
      if (unsafe === 'overrange child') first.p.opacity.v = 200;
      PM.touch(); PM.hist.clear();
      const render = () => PM.GL.renderToPixels(1, 320, 180, { transparent: true, mblur: false });
      const before = render();
      const source = PM.serialize(), selection = JSON.stringify(PM.sel);
      const edited = unsafe === 'overlapping children' ? PM.cmd('ungroupLayers')
        : PM.Edit.apply({ type: 'ungroup_layers', targets: [group.id] }, { origin: 'agent', label: 'Ungroup layers' });
      const after = render(); let maxDifference = 0;
      for (let index = 0; index < before.length; index++) maxDifference = Math.max(maxDifference, Math.abs(before[index] - after[index]));
      return { ok: edited.ok, message: edited.message, alpha: before[(90 * 320 + 160) * 4 + 3], maxDifference,
        unchanged: PM.serialize() === source, selectionUnchanged: JSON.stringify(PM.sel) === selection,
        history: PM.hist.list(), undo: PM.hist.undo(), groupType: PM.L(group.id).type };
    }, unsafe);
    if (unsafe === 'overlapping children') await expect(session.page.locator('.toast').filter({ hasText: 'Pre-compose this group' })).toBeVisible();
    expect(result.ok).toBe(false); expect(result.message).toContain('Pre-compose this group');
    expect(result.alpha).toBeGreaterThanOrEqual(127); expect(result.alpha).toBeLessThanOrEqual(129);
    expect(result.maxDifference).toBe(0); expect(result.unchanged).toBe(true); expect(result.selectionUnchanged).toBe(true);
    expect(result.history).toEqual([]); expect(result.undo).toBe(false); expect(result.groupType).toBe('group');
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
}

for (const animation of ['fade', 'clipped expression'] as const) {
  test(`Ungroup keeps a plain child ${animation}, actual pixels and Undo`, async ({ session }) => {
    await session.openEditor();
    await newProject(session.page, `Ungroup ${animation}`);
    const result = await session.page.evaluate(animation => {
      const PM = (window as any).PM; PM.pause(); PM.time = 1;
      const child = PM.mkLayer('shape', {
        d: { shape: 'rect', w: 80, h: 60, color: '#ffffff', radius: 0, stroke: 0 },
        p: { 'position.x': 160, 'position.y': 90, opacity: 80 },
      });
      PM.proj.layers = [child]; PM.ProjectIndex.invalidate(); PM.touch();
      const grouped = PM.Edit.apply({ type: 'group_layers', targets: [child.id] }, { origin: 'agent' });
      if (!grouped.ok) throw new Error(grouped.message);
      const group = PM.firstSel();
      if (animation === 'fade') group.p.opacity.kf = [PM.KF(0, 20, 'linear'), PM.KF(2, 80, 'linear')];
      else group.p.opacity.expr = 'time * 150 - 50';
      PM.touch(); PM.hist.clear();
      const times = [.5, 1.5];
      const render = (time: number) => PM.GL.renderToPixels(time, 320, 180, { transparent: true, mblur: false });
      const before = times.map(render);
      const difference = () => times.map((time, frame) => {
        const pixels = render(time); let max = 0;
        for (let index = 0; index < pixels.length; index++) max = Math.max(max, Math.abs(pixels[index] - before[frame][index]));
        return max;
      });
      const edited = PM.Edit.apply({ type: 'ungroup_layers', targets: [group.id] }, { origin: 'agent', label: 'Ungroup layers' });
      if (!edited.ok) throw new Error(edited.message);
      const after = difference(), removed = !PM.L(group.id), undo = PM.hist.undo(), restored = difference();
      const groupRestored = PM.L(group.id)?.type === 'group';
      const redo = PM.hist.redo(), redone = difference();
      return { centerAlpha: before.map(pixels => pixels[(90 * 320 + 160) * 4 + 3]), after, removed,
        undo, restored, groupRestored, redo, redone, history: PM.hist.list() };
    }, animation);
    const expectedAlpha = animation === 'fade' ? [.28, .52] : [.2, .8];
    result.centerAlpha.forEach((alpha, index) => expect(Math.abs(alpha / 255 - expectedAlpha[index]!)).toBeLessThan(.01));
    expect(result.removed).toBe(true); expect(result.undo).toBe(true); expect(result.groupRestored).toBe(true); expect(result.redo).toBe(true);
    expect(result.history).toEqual(['Ungroup layers']);
    [...result.after, ...result.restored, ...result.redone].forEach(delta => expect(delta).toBeLessThanOrEqual(2));
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
}
