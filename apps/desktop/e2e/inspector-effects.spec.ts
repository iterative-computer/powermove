import { expect, test } from './helpers/app';

test.beforeEach(async ({ session }) => { await session.openEditor(); });

test('a copied effect pastes onto the layer selected after copying', async ({ session }) => {
  const { page } = session;
  const ids = await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.replaceProject(PM.mkProject({ name: 'Simple effect paste QA', dur: 10 }));
    const source = PM.mkLayer('solid', { name: 'Source layer' });
    const destination = PM.mkLayer('solid', { name: 'Destination layer' });
    const effect = PM.mkEffect('blur');
    effect.p.amount.v = 32;
    source.fx.push(effect);
    PM.proj.layers.push(source, destination);
    PM.selectLayers(source.id);
    PM.hist.clear();
    PM.invalidate();
    return { source: source.id, destination: destination.id, effect: effect.id };
  });

  const effect = page.locator(`[data-effect-id="${ids.effect}"]`);
  await effect.click();
  await session.app.evaluate(({ Menu }) => Menu.getApplicationMenu()?.getMenuItemById('contextCopy')?.click());
  await expect(page.getByText('Copied 1 effect', { exact: true })).toBeVisible();
  await page.evaluate((destination) => (window as any).PM.selectLayers(destination), ids.destination);
  await expect(page.locator(`[data-inspector-layer="${ids.destination}"]`)).toBeVisible();
  await session.app.evaluate(({ Menu }) => Menu.getApplicationMenu()?.getMenuItemById('contextPaste')?.click());

  await expect.poll(() => page.evaluate((destination) => {
    const effects = (window as any).PM.L(destination).fx;
    return effects.map((candidate: any) => [candidate.type, candidate.p.amount.v]);
  }, ids.destination)).toEqual([['blur', 32]]);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

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

test('timeline controls occupy the ruler gutter without legacy navigation buttons', async ({ session }) => {
  const { page } = session;
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.selectLayers([]);
    PM.invalidate();
  });
  await expect(page.locator('#panel-inspector [data-inspector-header]')).toHaveCount(0);
  const graph = page.getByRole('button', { name: 'Graph editor (Shift+F3)', exact: true });
  await expect(graph.locator('[data-icon="bezier"]')).toHaveCount(1);
  expect(await graph.evaluate((button) => ({
    graphSlot: !!button.closest('.tl-graph-slot'),
    transport: !!button.closest('.tl-transport'),
    first: button.parentElement?.firstElementChild === button,
    previousGroup: button.parentElement?.previousElementSibling?.classList.contains('tl-transport')
  }))).toEqual({ graphSlot: true, transport: false, first: true, previousGroup: true });
  for (const name of ['Previous edge', 'Next edge', 'Frame entire composition (⇧F)', 'Loop']) {
    await expect(page.getByRole('button', { name, exact: true })).toHaveCount(0);
  }
  await expect(page.getByRole('slider', { name: 'Timeline zoom', exact: true })).toHaveCount(0);
  const geometry = await page.evaluate(() => {
    const PM = (window as any).PM;
    const timeline = PM.Kernel.services.get('timeline');
    const head = document.querySelector('#tl-head')!.getBoundingClientRect();
    const canvas = document.querySelector('#tl-canvas')!.getBoundingClientRect();
    const controls = [...document.querySelectorAll('#tl-head button, #tl-head input')]
      .map((element) => element.getBoundingClientRect());
    return {
      head: { x: head.x, y: head.y, width: head.width, height: head.height },
      canvas: { x: canvas.x, y: canvas.y },
      gutter: timeline.gut,
      ruler: timeline.ruler,
      controlsInside: controls.every((rect) => rect.left >= head.left && rect.right <= head.right
        && rect.top >= head.top && rect.bottom <= head.bottom)
    };
  });
  expect(Math.abs(geometry.head.x - geometry.canvas.x)).toBeLessThan(1.5);
  expect(Math.abs(geometry.head.y - geometry.canvas.y)).toBeLessThan(1.5);
  expect(Math.abs(geometry.head.width - geometry.gutter)).toBeLessThan(1.5);
  expect(Math.abs(geometry.head.height - geometry.ruler)).toBeLessThan(1.5);
  expect(geometry.controlsInside).toBe(true);
  expect(await page.locator('#tl-time').evaluate((element) => getComputedStyle(element).color))
    .toBe(await graph.evaluate((element) => getComputedStyle(element).color));
  const dragPoint = await page.evaluate(() => {
    const head = document.querySelector('#tl-head')!.getBoundingClientRect();
    const time = document.querySelector('#tl-time')!.getBoundingClientRect();
    const graph = document.querySelector<HTMLButtonElement>('button[title="Graph editor (Shift+F3)"]')!.getBoundingClientRect();
    return { x: (time.right + graph.left) / 2, y: head.top + head.height / 2 };
  });
  await page.mouse.move(dragPoint.x, dragPoint.y);
  await page.mouse.down();
  await page.mouse.move(dragPoint.x + 12, dragPoint.y + 8, { steps: 4 });
  await expect(page.locator('body')).toHaveClass(/panel-dragging/);
  await page.keyboard.press('Escape');
  await expect(page.locator('body')).not.toHaveClass(/panel-dragging/);
  await expect(page.locator('#panel-timeline')).toHaveCount(1);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('panel context menus stay wholly above or below the cursor', async ({ session }) => {
  const { page } = session;
  const assertPlacement = async (trigger: ReturnType<typeof page.locator>) => {
    const box = await trigger.boundingBox();
    if (!box) throw new Error('context-menu trigger is not visible');
    const cursor = { x: box.x + box.width / 2, y: box.y + box.height / 2 };
    await page.mouse.click(cursor.x, cursor.y, { button: 'right' });
    const menu = page.locator('.drop[role="menu"]');
    await expect(menu).toBeVisible();
    const placement = await menu.evaluate((element) => {
      const rect = element.getBoundingClientRect();
      return { top: rect.top, bottom: rect.bottom, side: element.getAttribute('data-side') };
    });
    expect(['above', 'below']).toContain(placement.side);
    if (placement.side === 'below') expect(placement.top).toBeGreaterThan(cursor.y);
    else expect(placement.bottom).toBeLessThan(cursor.y);
    await page.keyboard.press('Escape');
  };

  await assertPlacement(page.locator('#panel-assets > header'));
  await assertPlacement(page.locator('#panel-timeline .panel-move-handle'));
  expect(session.diagnostics.pageErrors).toEqual([]);
});
