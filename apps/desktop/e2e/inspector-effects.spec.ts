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

  // 4f5cf623d added the same effects to Properties and Effect Controls.
  const effect = page.locator(`#panel-inspector [data-effect-id="${ids.effect}"]`);
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

  const first = page.locator(`#panel-inspector [data-effect-id="${ids.first}"]`);
  const second = page.locator(`#panel-inspector [data-effect-id="${ids.second}"]`);
  await expect(first).toBeVisible();
  await first.click();
  await second.click({ modifiers: ['Meta'] });
  await expect(first).toHaveAttribute('aria-selected', 'true');
  await expect(second).toHaveAttribute('aria-selected', 'true');

  await page.keyboard.press('Meta+c');
  await page.keyboard.press('Meta+v');
  await expect(page.locator('#panel-inspector [data-effect-id]')).toHaveCount(4);
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
  await expect(page.locator('#panel-inspector [data-effect-id]')).toHaveCount(2);
  await first.click();
  await second.click({ modifiers: ['Meta'] });
  await page.keyboard.press('Delete');
  await expect(page.locator('#panel-inspector [data-effect-id]')).toHaveCount(0);
  await page.keyboard.press('Meta+z');
  await expect(page.locator('#panel-inspector [data-effect-id]')).toHaveCount(2);

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
  await expect(graph.locator('[data-icon="dopeGraph"]')).toHaveCount(1);
  expect(await graph.evaluate((button) => ({
    graphSlot: !!button.closest('.tl-graph-slot'),
    transport: !!button.closest('.tl-transport'),
    // Track-mode snapping remains in the DOM while hidden in layer mode.
    lastVisible: [...button.parentElement!.children]
      .filter(element => element.getClientRects().length > 0).at(-1) === button,
    previousGroup: button.parentElement?.previousElementSibling?.classList.contains('tl-transport')
  }))).toEqual({ graphSlot: true, transport: false, lastVisible: true, previousGroup: true });
  await expect(page.locator('#tl-head .tl-snap')).toBeHidden();
  for (const name of ['Previous edge', 'Next edge', 'Frame entire composition (⇧F)', 'Loop']) {
    await expect(page.getByRole('button', { name, exact: true })).toHaveCount(0);
  }
  await expect(page.getByRole('slider', { name: 'Timeline zoom', exact: true })).toHaveCount(0);
  const geometry = await page.evaluate(() => {
    const PM = (window as any).PM;
    const timeline = PM.Kernel.services.get('timeline');
    const head = document.querySelector('#tl-head')!.getBoundingClientRect();
    const canvas = document.querySelector('#tl-canvas')!.getBoundingClientRect();
    /* Graph options stays display:none until the graph editor opens, and a
       control with no box reports an all-zero rect that is not a position. */
    const controls = [...document.querySelectorAll('#tl-head button, #tl-head input')]
      .filter((element) => element.getClientRects().length > 0)
      .map((element) => element.getBoundingClientRect());
    return {
      head: { x: head.x, y: head.y, width: head.width, height: head.height },
      canvas: { x: canvas.x, y: canvas.y },
      gutter: timeline.gut,
      // The header sits over the time scale; the column headings strip is below it.
      ruler: timeline.ruler - (timeline.header || 0),
      controlsInside: controls.every((rect) => rect.left >= head.left && rect.right <= head.right
        && rect.top >= head.top && rect.bottom <= head.bottom)
    };
  });
  expect(Math.abs(geometry.head.x - geometry.canvas.x)).toBeLessThan(1.5);
  expect(Math.abs(geometry.head.y - geometry.canvas.y)).toBeLessThan(1.5);
  expect(Math.abs(geometry.head.width - geometry.gutter)).toBeLessThan(1.5);
  expect(Math.abs(geometry.head.height - geometry.ruler)).toBeLessThan(1.5);
  expect(geometry.controlsInside).toBe(true);
  // The timecode reads in the accent's text tone, as in the prototype.
  expect(await page.locator('#tl-time').evaluate((element) => getComputedStyle(element).color))
    .toBe(await page.evaluate(() => {
      const probe = document.createElement('span');
      probe.style.color = 'var(--accent-tx)';
      document.body.append(probe);
      const color = getComputedStyle(probe).color;
      probe.remove();
      return color;
    }));
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

// Panel menus use the shared overlay (06d25cf64). Assert the visible menu and its pointer anchor directly.
test('panel context menus open at the pointer position', async ({ session }) => {
  const { page } = session;
  await page.evaluate(() => (window as any).PM.Layout.activateTab?.('assets'));
  for (const selector of ['#panel-assets > header', '#panel-timeline .panel-move-handle']) {
    const box = await page.locator(selector).boundingBox();
    if (!box) throw new Error('context-menu trigger is not visible');
    const cursor = { x: box.x + box.width / 2, y: box.y + box.height / 2 };
    await page.mouse.click(cursor.x, cursor.y, { button: 'right' });
    const menu = page.locator('[data-svelte-overlay-menu] .menu-panel[data-level="0"]');
    await expect(menu).toBeVisible();
    await expect(menu.getByRole('menuitem', { name: 'Maximize Panel `', exact: true })).toBeVisible();
    const bounds = (await menu.boundingBox())!;
    // The overlay clamps within the window when the pointer is near an edge.
    const viewport = await page.evaluate(() => ({ width: innerWidth, height: innerHeight }));
    expect(Math.abs(bounds.x - Math.min(cursor.x, viewport.width - bounds.width - 6))).toBeLessThanOrEqual(1);
    expect(Math.abs(bounds.y - (cursor.y + bounds.height + 2 <= viewport.height - 6 ? cursor.y + 2 : cursor.y - bounds.height - 2))).toBeLessThanOrEqual(1);
    await page.keyboard.press('Escape');
    await expect(menu).toHaveCount(0);
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});
