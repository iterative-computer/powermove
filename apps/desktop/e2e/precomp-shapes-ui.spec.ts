import { expect, test } from './helpers/app';

/* Visible UI for selection-sized pre-comps, shape Size handles and corner Smoothing. */

async function scene(page: any) {
  return page.evaluate(async () => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ name: 'Shapes', w: 960, h: 540, fps: 30, dur: 6, bg: '#1d1d1f' });
    const card = PM.mkLayer('shape', { name: 'Card', from: .5, dur: 4, d: { shape: 'rect', w: 260, h: 160, radius: 28, smoothing: 0, color: '#e8e2cf', stroke: 0 }, p: { 'position.x': 330, 'position.y': 250, rotation: -8 } }, project);
    const pill = PM.mkLayer('shape', { name: 'Pill', from: 1, dur: 3, d: { shape: 'rect', w: 240, h: 72, radius: 36, smoothing: 60, color: '#a5a2ff', stroke: 0 }, p: { 'position.x': 610, 'position.y': 330 } }, project);
    const dot = PM.mkLayer('shape', { name: 'Dot', d: { shape: 'ellipse', w: 90, h: 90, color: '#ff9f6b', stroke: 0 }, p: { 'position.x': 760, 'position.y': 170 } }, project);
    project.layers = [card, pill, dot];
    PM.replaceProject(project);
    PM.setTime(2, { raw: true, force: true });
    PM.selectLayers([]);
    const viewer = PM.Kernel.services.get('viewer');
    PM.Kernel.services.get('tool').setTool('select');
    viewer.fit = true; viewer.pan = [0, 0]; viewer.layout();
    await new Promise<void>((r) => requestAnimationFrame(() => requestAnimationFrame(() => r())));
    return { card: card.id, pill: pill.id, dot: dot.id };
  });
}

test('Pre-compose can fit the new composition to the selection, and it stays in place', async ({ session }) => {
  await session.openEditor();
  const page = session.page;
  const ids = await scene(page);
  const render = () => page.evaluate(() => Array.from((window as any).PM.GL.renderToPixels(2, 480, 270, { opaque: true, mblur: false, exporting: true }) as Uint8Array));
  const before = await render();
  await page.evaluate((sel: string[]) => { const PM = (window as any).PM; PM.selectLayers(sel); PM.cmd('precompose'); }, [ids.card, ids.pill]);
  const modal = page.locator('.modal').last();
  const sizeRow = modal.locator('[data-precompose-size]');
  await expect(sizeRow).toContainText('Fit size to layers');
  await expect(modal.locator('.precompose-summary')).toHaveText('New composition: 960 × 540 px, 6 s');
  await sizeRow.getByRole('radio', { name: 'On' }).click();
  await expect(modal.locator('.precompose-summary')).toHaveText(/^New composition: \d+ × \d+ px, 6 s$/);
  const summary = await modal.locator('.precompose-summary').textContent();
  await modal.getByRole('button', { name: 'Pre-compose', exact: true }).click();
  await expect(modal).toBeHidden();
  const result = await page.evaluate(() => {
    const PM = (window as any).PM, use = PM.proj.layers.find((l: any) => l.type === 'precomp');
    const comp = PM.proj.comps[use.d.comp];
    return { w: comp.w, h: comp.h, layers: PM.proj.layers.length, history: PM.hist.list().at(-1) };
  });
  expect(summary).toContain(`${result.w} × ${result.h} px`);
  expect(result.w).toBeLessThan(960); expect(result.h).toBeLessThan(540);
  expect(result.layers).toBe(2);
  const after = await render();
  let max = 0; for (let i = 0; i < before.length; i++) max = Math.max(max, Math.abs(before[i]! - after[i]!));
  expect(max).toBeLessThanOrEqual(2);
  expect(await page.evaluate(() => (window as any).PM.hist.undo())).toBe(true);
  expect(await page.evaluate(() => (window as any).PM.proj.layers.length)).toBe(3);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('Selection handles on a shape change its Size, with Shift and Option, in one undo step', async ({ session }) => {
  await session.openEditor();
  const page = session.page;
  const ids = await scene(page);
  await page.evaluate((id: string) => (window as any).PM.selectLayers([id]), ids.card);
  const corner = (name: string) => page.evaluate(([id, name]: string[]) => {
    const PM = (window as any).PM, viewer = PM.Kernel.services.get('viewer');
    const L = PM.L(id), T = PM.time; PM.beginEval(T);
    const b = PM.GL.bounds(L, T), m = PM.worldMatrix(L, T);
    const fx = name.includes('w') ? 0 : name.includes('e') ? 1 : .5, fy = name.includes('n') ? 0 : name.includes('s') ? 1 : .5;
    const x = b.x0 + fx * b.w, y = b.y0 + fy * b.h;
    return { x: m[0] * x + m[2] * y + m[4], y: m[1] * x + m[3] * y + m[5], shown: viewer.shown };
  }, [ids.card, name]);
  const client = async (point: { x: number; y: number; shown: number }) => {
    const box = (await page.locator('#stage-inner').boundingBox())!;
    return { x: box.x + point.x * point.shown, y: box.y + point.y * point.shown };
  };
  const state = () => page.evaluate((id: string) => {
    const PM = (window as any).PM, L = PM.L(id), content = PM.resolveContent(L, PM.time);
    return { w: content.w, h: content.h, sx: L.p['scale.x'].v, sy: L.p['scale.y'].v };
  }, ids.card);

  const fixedBefore = await corner('nw');
  const start = await client(await corner('se'));
  await page.mouse.move(start.x, start.y); await page.mouse.down();
  await page.mouse.move(start.x + 30, start.y + 20, { steps: 4 });
  await expect.poll(() => page.evaluate(() => (window as any).PM.Kernel.services.get('viewer').sizeReadout)).toMatchObject({ w: expect.any(Number) });
  await page.mouse.move(start.x + 60, start.y + 30, { steps: 4 });
  await page.mouse.up();
  const resized = await state();
  expect(resized.w).toBeGreaterThan(260); expect(resized.h).toBeGreaterThan(160);
  expect([resized.sx, resized.sy]).toEqual([100, 100]);
  const fixedAfter = await corner('nw');
  expect(fixedAfter.x).toBeCloseTo(fixedBefore.x, 1); expect(fixedAfter.y).toBeCloseTo(fixedBefore.y, 1);
  expect(await page.evaluate(() => (window as any).PM.Kernel.services.get('viewer').sizeReadout ?? null)).toBeNull();
  expect(await page.evaluate(() => (window as any).PM.hist.list().at(-1))).toBe('Resize shape');
  expect(await page.evaluate(() => (window as any).PM.hist.undo())).toBe(true);
  expect(await state()).toMatchObject({ w: 260, h: 160 });

  const centerBefore = await corner('c');
  const edge = await client(await corner('e'));
  await page.keyboard.down('Shift'); await page.keyboard.down('Alt');
  await page.mouse.move(edge.x, edge.y); await page.mouse.down();
  await page.mouse.move(edge.x + 40, edge.y + 3, { steps: 6 });
  await page.mouse.up();
  await page.keyboard.up('Alt'); await page.keyboard.up('Shift');
  const proportional = await state();
  expect(proportional.w / proportional.h).toBeCloseTo(260 / 160, 2);
  expect(proportional.w).toBeGreaterThan(260);
  const centerAfter = await corner('c');
  expect(centerAfter.x).toBeCloseTo(centerBefore.x, 1); expect(centerAfter.y).toBeCloseTo(centerBefore.y, 1);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('Smoothing sits beside the corner radius, is keyframeable, and only shows for rectangles', async ({ session }) => {
  await session.openEditor();
  const page = session.page;
  const ids = await scene(page);
  await page.evaluate((id: string) => (window as any).PM.selectLayers([id]), ids.card);
  const inspector = page.locator('#panel-inspector');
  const control = inspector.locator('[data-smoothing-control]');
  await expect(control).toBeVisible();
  await expect(control).toHaveAttribute('title', /iOS/);
  const row = inspector.locator('.row', { has: page.locator('[data-smoothing-control]') });
  await expect(row.locator('.k')).toHaveText('Smoothing');
  const field = control.locator('input.num');
  await field.click(); await field.fill('60'); await field.press('Enter');
  await expect.poll(() => page.evaluate((id: string) => (window as any).PM.resolveContent((window as any).PM.L(id), 2).smoothing, ids.card)).toBe(60);
  await row.hover();
  await row.locator('.stopwatch').click();
  await expect.poll(() => page.evaluate((id: string) => (window as any).PM.L(id).d.smoothing?.kf?.length ?? 0, ids.card)).toBe(1);
  await page.evaluate((id: string) => (window as any).PM.selectLayers([id]), ids.dot);
  await expect(inspector.locator('[data-smoothing-control]')).toHaveCount(0);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
