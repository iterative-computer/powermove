import { expect, test } from './helpers/app';

test('effect rows select, copy, paste, delete, and undo as editable source transactions', async ({ session }) => {
  const { page } = session;
  const ids = await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.replaceProject(PM.mkProject({ name: 'Effect clipboard QA', dur: 10 }));
    const layer = PM.mkLayer('solid', { name: 'Effect clipboard layer' });
    const first = PM.mkEffect('blur');
    const second = PM.mkEffect('blur');
    first.on = false;
    first.open = false;
    first.p.amount.v = 12;
    first.p.amount.expr = 'value * 2';
    first.p.amount.kf = [PM.KF(1, 18, 'easeInOut')];
    second.p.amount.v = 24;
    layer.fx.push(first, second);
    PM.proj.layers.push(layer);
    PM.selectLayers(layer.id);
    PM.hist.clear();
    PM.invalidate();
    return { layer: layer.id, first: first.id, second: second.id, firstKey: first.p.amount.kf[0].i };
  });

  const first = page.locator(`[data-effect-id="${ids.first}"]`);
  const second = page.locator(`[data-effect-id="${ids.second}"]`);
  await expect(first).toBeVisible();
  await first.click();
  await second.click({ modifiers: ['Meta'] });
  await expect(first).toHaveAttribute('aria-selected', 'true');
  await expect(second).toHaveAttribute('aria-selected', 'true');

  await page.keyboard.press('Meta+c');
  await page.keyboard.press('Meta+v');
  await expect(page.locator('[data-effect-id]')).toHaveCount(4);
  const copiedState = await page.evaluate(({ layer }) => {
    const effects = (window as any).PM.L(layer).fx;
    return effects.map((effect: any) => [effect.on, effect.open, effect.p.amount.v, effect.p.amount.expr]);
  }, ids);
  expect(copiedState.slice(2)).toEqual(copiedState.slice(0, 2));
  expect(copiedState[0]).toMatchObject([false, expect.any(Boolean), 12, 'value * 2']);
  expect(copiedState[1]).toMatchObject([true, expect.any(Boolean), 24, null]);
  const identity = await page.evaluate(({ layer }) => {
    const effects = (window as any).PM.L(layer).fx;
    const effectIds = effects.map((effect: any) => effect.id);
    const keyIds = effects.flatMap((effect: any) => effect.p.amount.kf.map((key: any) => key.i));
    return { effects: [effectIds.length, new Set(effectIds).size], keys: [keyIds.length, new Set(keyIds).size] };
  }, ids);
  expect(identity.effects[0]).toBe(identity.effects[1]);
  expect(identity.keys[0]).toBe(identity.keys[1]);

  await page.keyboard.press('Meta+z');
  await expect(page.locator('[data-effect-id]')).toHaveCount(2);
  await first.click();
  await second.click({ modifiers: ['Meta'] });
  await page.keyboard.press('Delete');
  await expect(page.locator('[data-effect-id]')).toHaveCount(0);
  await page.keyboard.press('Meta+z');
  await expect(page.locator('[data-effect-id]')).toHaveCount(2);

  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('composition properties do not repeat the panel title and the Bézier control sits by timecode', async ({ session }) => {
  const { page } = session;
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.selectLayers([]);
    PM.invalidate();
  });
  await expect(page.locator('#panel-inspector [data-inspector-header]')).toHaveCount(0);
  const graph = page.getByRole('button', { name: 'Graph editor (G)', exact: true });
  await expect(graph.locator('[data-icon="bezier"]')).toHaveCount(1);
  expect(await graph.evaluate((button) => ({
    transport: !!button.closest('.tl-transport'),
    next: button.nextElementSibling?.id
  }))).toEqual({ transport: true, next: 'tl-time' });
  expect(session.diagnostics.pageErrors).toEqual([]);
});
