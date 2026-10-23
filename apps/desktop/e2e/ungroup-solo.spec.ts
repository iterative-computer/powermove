import { expect, test } from './helpers/app';

for (const soloOn of ['group', 'child'] as const) {
  test(`Ungroup preserves actual pixels with Solo on the ${soloOn}`, async ({ session }) => {
    await session.openEditor();
    await session.page.waitForFunction(() => !(window as any).PM.projectLoading);
    await session.page.evaluate(() => (window as any).PM.cmd('newProject'));
    const modal = session.page.locator('.modal');
    await modal.getByLabel('Project name').fill(`Ungroup ${soloOn} Solo`);
    await modal.getByLabel('Width in pixels').fill('320');
    await modal.getByLabel('Height in pixels').fill('180');
    await modal.getByLabel('Duration in seconds').fill('2');
    await modal.getByRole('button', { name: 'Create', exact: true }).click();
    await session.page.waitForFunction(name => {
      const PM = (window as any).PM;
      return PM.proj.name === name && !PM.projectLoading && PM.GL?.gl;
    }, `Ungroup ${soloOn} Solo`);

    const result = await session.page.evaluate(soloOn => {
      const PM = (window as any).PM; PM.pause(); PM.time = .5;
      const shape = (x: number, color: string) => PM.mkLayer('shape', {
        d: { shape: 'rect', w: 40, h: 50, color, radius: 0, stroke: 0 }, p: { 'position.x': x, 'position.y': 90 },
      });
      const first = shape(90, '#ffffff'), second = shape(150, '#ff6040'), outside = shape(260, '#40a0ff');
      PM.proj.layers = [first, second, outside]; PM.ProjectIndex.invalidate(); PM.touch();
      const group = PM.groupLayers([first.id, second.id]);
      const solo = PM.Edit.apply({ type: 'set_layer', target: soloOn === 'group' ? group.id : first.id, patch: { solo: true } }, { origin: 'timeline' });
      if (!solo.ok) throw new Error(solo.message);
      PM.hist.clear();
      const render = () => PM.GL.renderToPixels(.5, 320, 180, { transparent: true, mblur: false });
      const before = render(), source = PM.serialize(), selection = JSON.stringify(PM.sel);
      const difference = () => {
        const pixels = render(); let max = 0;
        for (let index = 0; index < pixels.length; index++) max = Math.max(max, Math.abs(pixels[index] - before[index]));
        return max;
      };
      const edit = PM.cmd('ungroupLayers'), after = difference();
      if (!edit.ok) return { ok: edit.ok, message: edit.message, after, unchanged: PM.serialize() === source,
        selected: JSON.stringify(PM.sel) === selection, history: PM.hist.list(), undo: PM.hist.undo(),
        firstAlpha: before[(90 * 320 + 90) * 4 + 3], secondAlpha: before[(90 * 320 + 150) * 4 + 3],
        outsideAlpha: before[(90 * 320 + 260) * 4 + 3], outsideActive: PM.active(outside, .5) };
      const removed = !PM.L(group.id), childSolo = PM.L(first.id).solo, undo = PM.hist.undo(), restored = difference();
      const redo = PM.hist.redo(), redone = difference();
      return { ok: edit.ok, after, removed, childSolo, undo, restored, redo, redone, history: PM.hist.list(),
        firstAlpha: before[(90 * 320 + 90) * 4 + 3], secondAlpha: before[(90 * 320 + 150) * 4 + 3],
        outsideAlpha: before[(90 * 320 + 260) * 4 + 3], outsideActive: PM.active(PM.L(outside.id), .5) };
    }, soloOn);
    expect(result.firstAlpha).toBe(255); expect(result.secondAlpha).toBe(soloOn === 'group' ? 255 : 0);
    expect(result.outsideAlpha).toBe(0); expect(result.outsideActive).toBe(true); expect(result.after).toBeLessThanOrEqual(2);
    if (soloOn === 'group') {
      await expect(session.page.locator('.toast').filter({ hasText: 'Turn off Solo before ungrouping this group' })).toBeVisible();
      expect(result.ok).toBe(false); expect(result.message).toContain('Turn off Solo');
      expect(result.unchanged).toBe(true); expect(result.selected).toBe(true); expect(result.history).toEqual([]); expect(result.undo).toBe(false);
    } else {
      expect(result.ok).toBe(true); expect(result.removed).toBe(true); expect(result.childSolo).toBe(true);
      expect(result.undo).toBe(true); expect(result.redo).toBe(true); expect(result.history).toEqual(['Ungroup layers']);
      expect(result.restored).toBeLessThanOrEqual(2); expect(result.redone).toBeLessThanOrEqual(2);
    }
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
}
