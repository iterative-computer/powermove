import { expect, test } from './helpers/app';

/* Tester feedback (Figuate, 2026-11-07): double-clicking a pre-comp's bar
   should open it, the composition path belongs at the top of the viewer,
   and scrolling or middle-dragging over a 3D scene must not switch views. */

async function precompProject(page: any) {
  await page.evaluate(() => {
    const PM = (window as any).PM;
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: PM.mkProject({ name: 'Untitled' }) }));
    PM.ProjectsScreen.hide();
  });
  await page.waitForFunction(() => { const PM = (window as any).PM; return Boolean(PM?.GL?.gl && PM.Kernel.services.get('timeline')?.rows); });
  return page.evaluate(() => {
    const PM = (window as any).PM;
    const rect = PM.mkLayer('shape', { name: 'Rect', d: { w: 600, h: 320, color: '#E6E0CC' } });
    PM.proj.layers = [rect]; PM.invalidate();
    const root = PM.Comps.active();
    PM.Comps.precompose([rect.id], { name: 'Pre-comp 1' });
    const layer = PM.proj.layers[0];
    PM.selectLayers([]); PM.invalidate();
    return { root, child: layer.d.comp as string, layer: layer.id as string };
  });
}

const crumbs = (page: any) => page.locator('#viewer-path .viewer-crumb');

test('double-clicking a pre-comp bar opens it; the viewer path steps back up and down', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  const ids = await precompProject(page);
  await expect(crumbs(page)).toHaveText(['Untitled']);
  await expect(page.locator('#viewer-path [aria-current="page"]')).toHaveText('Untitled');

  const bar = await page.evaluate(() => {
    const PM = (window as any).PM, t = PM.Kernel.services.get('timeline'), r = t.cv.getBoundingClientRect();
    return { x: r.left + t.gut + 120, y: r.top + t.ruler + t.row / 2 - t.scrollY };
  });
  await page.mouse.dblclick(bar.x, bar.y);
  await expect.poll(() => page.evaluate(() => (window as any).PM.Comps.active())).toBe(ids.child);
  // The bar's double-click opened it without moving or trimming the layer.
  await expect(crumbs(page)).toHaveText(['Untitled', 'Pre-comp 1']);
  await expect(page.locator('#viewer-path [aria-current="page"]')).toHaveText('Pre-comp 1');

  // The header sits above the canvas, never over it.
  const geometry = await page.evaluate(() => {
    const head = document.querySelector('#viewer-head')!.getBoundingClientRect();
    const stage = document.querySelector('#stage')!.getBoundingClientRect();
    return { headBottom: head.bottom, stageTop: stage.top, height: head.height };
  });
  expect(geometry.stageTop).toBeGreaterThanOrEqual(geometry.headBottom - 0.5);
  expect(geometry.height).toBeGreaterThan(20);

  await page.locator('#viewer-path button.viewer-crumb', { hasText: 'Untitled' }).click();
  await expect.poll(() => page.evaluate(() => (window as any).PM.Comps.active())).toBe(ids.root);
  // Pre-comp 1 stays, dimmed, to the right so you can step back down.
  await expect(crumbs(page)).toHaveText(['Untitled', 'Pre-comp 1']);
  await expect(page.locator('#viewer-path .viewer-crumb.ahead')).toHaveText('Pre-comp 1');
  await page.locator('#viewer-path .viewer-crumb.ahead').click();
  await expect.poll(() => page.evaluate(() => (window as any).PM.Comps.active())).toBe(ids.child);

  // The timeline tabs still work and agree with the path.
  await page.locator(`.tl-comp-tab[data-comp-id="${ids.root}"]`).dispatchEvent('pointerdown', { button: 0 });
  await expect.poll(() => page.evaluate(() => (window as any).PM.Comps.active())).toBe(ids.root);
  await expect(page.locator('#viewer-path [aria-current="page"]')).toHaveText('Untitled');

  // Double-clicking the pre-comp in the viewer opens it too, even on its
  // transparent areas, as the first click starts a (zero-length) move.
  const center = await page.evaluate(() => { const r = document.querySelector('#gl')!.getBoundingClientRect(); return { x: r.left + r.width / 2, y: r.top + r.height / 2 }; });
  await page.mouse.dblclick(center.x, center.y);
  await expect.poll(() => page.evaluate(() => (window as any).PM.Comps.active())).toBe(ids.child);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('Enter renames the selected layer; a pre-comp name double-click opens it instead', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  const ids = await precompProject(page);
  const row = await page.evaluate(() => {
    const PM = (window as any).PM, t = PM.Kernel.services.get('timeline'), r = t.cv.getBoundingClientRect();
    return { x: r.left + 160, y: r.top + t.ruler + t.row / 2 - t.scrollY };
  });
  await page.mouse.click(row.x, row.y);
  await expect.poll(() => page.evaluate(() => (window as any).PM.sel.layers)).toEqual([ids.layer]);
  await page.keyboard.press('Enter');
  const input = page.locator('#tl-canvas-wrap input');
  await expect(input).toBeVisible();
  await expect(input).toHaveValue('Pre-comp 1');
  await input.fill('Intro card');
  await input.press('Enter');
  await expect.poll(() => page.evaluate((id: string) => (window as any).PM.L(id).name, ids.layer)).toBe('Intro card');
  expect(await page.evaluate(() => (window as any).PM.Comps.active())).toBe(ids.root);

  await page.mouse.dblclick(row.x, row.y);
  await expect.poll(() => page.evaluate(() => (window as any).PM.Comps.active())).toBe(ids.child);
  await expect(page.locator('#tl-canvas-wrap input')).toHaveCount(0);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('with a 3D layer, scrolling and middle-dragging keep the Active Camera view and pan like 2D', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await page.evaluate(() => {
    const PM = (window as any).PM, api = PM.Kernel.api('comp-navigation-proof');
    api.scene3d.edit({ operation: 'add_object', object: { id: 'model', source: { primitive: 'box' }, material: { p: { color: '#FF7733' } } } });
    PM.selectLayers(['model']); PM.invalidate();
  });
  const view = () => page.evaluate(() => (window as any).PM.Kernel.api('comp-navigation-proof').scene3d.getView());
  const pan = () => page.evaluate(() => [...(window as any).PM.Kernel.services.get('viewer').pan]);
  await expect(page.getByRole('tab', { name: 'Active Camera', exact: true })).toHaveAttribute('aria-selected', 'true');
  // The 3D tools live in the viewer's header and footer, off the canvas.
  await expect(page.locator('#viewer-head #composition-3d-controls')).toBeVisible();
  await expect(page.locator('#viewer-foot #composition-3d-view')).toBeVisible();
  await expect(page.getByRole('tab', { name: 'Free View', exact: true })).toHaveAttribute('title', /Look around the 3D scene/);

  const box = await page.locator('#stage').boundingBox();
  const p = { x: box!.x + box!.width * 0.2, y: box!.y + box!.height * 0.8 };
  const before = await pan();
  await page.mouse.move(p.x, p.y); await page.mouse.down({ button: 'middle' });
  await page.mouse.move(p.x + 80, p.y - 30, { steps: 6 }); await page.mouse.up({ button: 'middle' });
  expect(await view()).toBe('camera');
  const afterDrag = await pan();
  expect(afterDrag).not.toEqual(before);

  // A trackpad two-finger scroll (fractional, two-axis) pans the same way.
  await page.mouse.move(p.x, p.y);
  await page.evaluate(({ x, y }) => {
    document.querySelector('#stage')!.dispatchEvent(new WheelEvent('wheel', { clientX: x, clientY: y, deltaX: 12.5, deltaY: 18.25, deltaMode: 0, bubbles: true, cancelable: true }));
  }, p);
  expect(await view()).toBe('camera');
  expect(await pan()).not.toEqual(afterDrag);
  await page.keyboard.down('Alt'); await page.mouse.wheel(0, 120); await page.keyboard.up('Alt');
  expect(await view()).toBe('camera');

  // Free View is explicit; there a middle-drag orbits, and Active Camera returns.
  await page.getByRole('tab', { name: 'Free View', exact: true }).click();
  expect(await view()).toBe('editor');
  await page.mouse.move(p.x, p.y); await page.mouse.down({ button: 'middle' });
  await page.mouse.move(p.x + 80, p.y - 30, { steps: 6 }); await page.mouse.up({ button: 'middle' });
  expect(await view()).toBe('editor');
  await page.getByRole('tab', { name: 'Active Camera', exact: true }).click();
  expect(await view()).toBe('camera');

  // Picking a navigation tool switches to Free View visibly.
  const navigation = page.getByRole('combobox', { name: '3D navigation', exact: true });
  await navigation.click();
  await page.getByRole('option', { name: 'Orbit', exact: true }).click();
  await expect(page.getByRole('tab', { name: 'Free View', exact: true })).toHaveAttribute('aria-selected', 'true');
  expect(session.diagnostics.pageErrors).toEqual([]);
});
