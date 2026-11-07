import { test, expect } from './helpers/app';
import { allFeatures } from '../src/shared/figma/__fixtures__/all-features';
import type { FigmaNode } from '../src/shared/figma/doc';

/* Properties controls for the editable layer data a Figma import produces
   (docs/figma-layer-data.md): each edit goes through set_property (or a
   structural set_content), changes the render, undoes in one step, and
   animatable fields key from their diamond. */
test.use({ desktopLaunchOptions: { env: { POWERMOVE_TEST_FIGMA_AUTOSTART: '1' } } });

async function connect(session: any) {
  const page = session.page;
  await session.app.evaluate(({ Menu }: any) => Menu.getApplicationMenu().getMenuItemById('connectFigma').click());
  await expect(page.getByRole('button', { name: 'Copy Plugin Path' })).toBeEnabled();
  const pair = fetch('http://localhost:47832/pair', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ code: '246810' }) });
  await page.getByRole('button', { name: 'Allow', exact: true }).click();
  const { token } = (await (await pair).json()) as any;
  await page.getByRole('button', { name: 'Done', exact: true }).click();
  return async (nodes: FigmaNode[]) => {
    await page.evaluate(() => {
      const P = (window as any).PM;
      P.replaceProject(P.mkProject({ w: 320, h: 240, dur: 4, bg: '#272727' }));
      P.setTime(0);
      P.hist.clear();
    });
    const response = await fetch('http://localhost:47832/send', {
      method: 'POST',
      headers: { 'content-type': 'application/json', authorization: 'Bearer ' + token },
      body: JSON.stringify({ ...allFeatures(), nodes }),
    });
    expect(response.status).toBe(200);
    return response.json() as Promise<any>;
  };
}

const node = (name: string, x = 40) => {
  const n = structuredClone(allFeatures().nodes.find((candidate) => candidate.name === name)!);
  n.absoluteTransform = [1, 0, 0, 1, x, 30];
  return n;
};

/** Selects the imported leaf (not its group or clipping shape) and starts logging edit commands. */
async function selectLeaf(page: any, name?: string) {
  await page.evaluate((name?: string) => {
    const P = (window as any).PM;
    const leaf = name ? P.proj.layers.find((l: any) => l.name === name)
      : [...P.proj.layers].reverse().find((l: any) => l.type !== 'group' && !P.proj.layers.some((o: any) => o.matteSource === l.id));
    P.selectLayers([leaf.id]);
    P.bus.emit('layers');
    P.dismissToast?.('figma-import');
    const log: any[] = ((window as any).__commands = []);
    if (!P.Edit.__logged) {
      const { apply, dispatch } = P.Edit;
      P.Edit.apply = (input: any, ...rest: any[]) => { (window as any).__commands.push(...[].concat(input)); return apply.call(P.Edit, input, ...rest); };
      P.Edit.dispatch = (input: any, ...rest: any[]) => { (window as any).__commands.push(...[].concat(input)); return dispatch.call(P.Edit, input, ...rest); };
      P.Edit.__logged = true;
    }
    void log;
  }, name);
}

const layerData = (page: any, expression: string) =>
  page.evaluate((expression: string) => {
    const P = (window as any).PM;
    const layer = P.L(P.sel.layers[0]);
    return new Function('layer', 'P', `return (${expression});`)(layer, P);
  }, expression);
const commands = (page: any) => page.evaluate(() => (window as any).__commands.map((c: any) => `${c.type}${c.path ? ' ' + c.path : ''}`));
const frame = (page: any) =>
  page.evaluate(() => {
    const P = (window as any).PM;
    P.rasterClear();
    const data = P.renderFrameTo(0, 320, 240).getContext('2d').getImageData(0, 0, 320, 240).data;
    let hash = 0;
    for (let i = 0; i < data.length; i += 7) hash = (hash * 31 + data[i]) | 0;
    return hash;
  });
const undo = (page: any) => page.evaluate(() => (window as any).PM.hist.undo());
const content = (page: any) => page.locator('#panel-inspector .content-rows');
async function typeInto(field: any, value: string) {
  await field.focus();
  await field.fill(value);
  await field.press('Enter');
}

test('gradient fill: stops, angle, solid switch and keyframes', async ({ session }) => {
  const { page } = session;
  await session.openEditor();
  const send = await connect(session);
  await send([node('LINEAR gradient')]);
  await selectLeaf(page);
  const rows = content(page);
  await expect(rows.getByRole('button', { name: 'Fill gradient', exact: true })).toHaveAttribute('aria-pressed', 'true');
  const before = await frame(page);

  await rows.getByRole('button', { name: 'Add fill gradient stop' }).click();
  expect(await layerData(page, 'layer.d.fillGradient.stops.length')).toBe(4);
  expect(await commands(page)).toEqual(['set_content']);
  await undo(page);
  expect(await layerData(page, 'layer.d.fillGradient.stops.length')).toBe(3);

  await page.evaluate(() => { (window as any).__commands.length = 0; });
  await typeInto(rows.getByRole('spinbutton', { name: 'Angle', exact: true }), '90');
  expect((await commands(page)).every((c: string) => c.startsWith('set_property c.fillGradient.transform.'))).toBe(true);
  await expect(rows.getByRole('spinbutton', { name: 'Angle', exact: true })).toHaveValue('90°');
  const turned = await frame(page);
  expect(turned).not.toBe(before);
  await undo(page);
  expect(await frame(page)).toBe(before);

  // Selecting a handle on the strip picks the stop the rows below edit.
  const historyBefore = await page.evaluate(() => (window as any).PM.hist.mark().index);
  await page.locator('#panel-inspector [data-gradient-stops="fill"] .ramp-stop').first().click();
  expect(await page.evaluate(() => (window as any).PM.hist.mark().index)).toBe(historyBefore);
  await expect(rows.getByRole('spinbutton', { name: 'Stop position', exact: true })).toHaveValue('0%');
  await typeInto(rows.getByRole('spinbutton', { name: 'Stop position', exact: true }), '25');
  expect(await layerData(page, 'layer.d.fillGradient.stops[0].position.v')).toBe(0.25);
  await undo(page);

  // The Angle diamond keys the whole transform at the playhead.
  await rows.getByRole('button', { name: 'Add keyframe for Angle' }).click();
  expect(await layerData(page, '[0,1,2,3,4,5].map(i => layer.d.fillGradient.transform[i].kf.length)')).toEqual([1, 1, 1, 1, 1, 1]);
  await rows.getByRole('button', { name: 'Add keyframe for Stop color' }).click();
  expect(await layerData(page, 'layer.d.fillGradient.stops[0].color.kf.length')).toBe(1);

  await rows.getByRole('button', { name: 'Fill gradient', exact: true }).click();
  expect(await layerData(page, '[layer.d.fillGradient, layer.d.color]')).toEqual([null, '#FF3300FF']);
  await expect(rows.getByRole('button', { name: 'Fill gradient', exact: true })).toHaveAttribute('aria-pressed', 'false');
  await undo(page);
  expect(await layerData(page, 'layer.d.fillGradient.stops.length')).toBe(3);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('dragging a gradient stop is one undo step', async ({ session }) => {
  const { page } = session;
  await session.openEditor();
  const send = await connect(session);
  await send([node('LINEAR gradient')]);
  await selectLeaf(page);
  const handle = page.locator('#panel-inspector [data-gradient-stops="fill"] .ramp-stop').nth(1);
  const box = (await handle.boundingBox())!;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(box.x + 30, box.y + box.height / 2, { steps: 5 });
  await page.mouse.up();
  const moved = await layerData(page, 'layer.d.fillGradient.stops[1].position.v');
  expect(moved).toBeGreaterThan(0.45);
  expect(await page.evaluate(() => (window as any).__commands.every((c: any) => c.type === 'set_property'))).toBe(true);
  await undo(page);
  expect(await layerData(page, 'layer.d.fillGradient.stops[1].position.v ?? layer.d.fillGradient.stops[1].position')).toBe(0.4);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('stroke position and dashes, arc sweep and image mode', async ({ session }) => {
  const { page } = session;
  await session.openEditor();
  const send = await connect(session);
  const rows = content(page);

  await send([node('INSIDE stroke')]);
  await selectLeaf(page);
  const inside = await frame(page);
  await rows.getByRole('button', { name: 'Outside', exact: true }).click();
  expect(await commands(page)).toEqual(['set_property c.strokeStyle.align']);
  expect(await layerData(page, 'layer.d.strokeStyle.align.v')).toBe('outside');
  expect(await frame(page)).not.toBe(inside);
  await undo(page);
  expect(await layerData(page, 'layer.d.strokeStyle.align.v ?? layer.d.strokeStyle.align')).toBe('inside');
  await typeInto(rows.getByRole('spinbutton', { name: 'Dash', exact: true }), '6');
  expect(await layerData(page, 'layer.d.strokeStyle.dashes')).toEqual([6, 6]);
  await expect(rows.getByRole('spinbutton', { name: 'Gap', exact: true })).toHaveValue('6px');
  await undo(page);
  expect(await layerData(page, 'layer.d.strokeStyle.dashes')).toEqual([]);

  await send([node('Arc donut')]);
  await selectLeaf(page);
  const donut = await frame(page);
  await typeInto(rows.getByRole('spinbutton', { name: 'Sweep', exact: true }), '50');
  expect(await commands(page)).toEqual(['set_property c.arc.end']);
  expect(await layerData(page, 'layer.d.arc.end.v - layer.d.arc.start')).toBeCloseTo(Math.PI, 5);
  expect(await frame(page)).not.toBe(donut);
  await undo(page);
  expect(await layerData(page, 'layer.d.arc.end.v ?? layer.d.arc.end')).toBe(5.5);
  await rows.getByRole('button', { name: 'Add keyframe for Sweep' }).click();
  expect(await layerData(page, 'layer.d.arc.end.kf.length')).toBe(1);

  await send([node('Image FILL')]);
  await selectLeaf(page);
  await expect(rows.getByRole('button', { name: 'Fill', exact: true })).toHaveAttribute('aria-pressed', 'true');
  const filled = await frame(page);
  await rows.getByRole('button', { name: 'Tile', exact: true }).click();
  expect(await commands(page)).toEqual(['set_property c.imageMapping.mode']);
  await expect(rows.getByRole('spinbutton', { name: 'Tile scale', exact: true })).toHaveValue('50%');
  expect(await frame(page)).not.toBe(filled);
  await undo(page);
  expect(await layerData(page, 'layer.d.imageMapping.mode.v ?? layer.d.imageMapping.mode')).toBe('fill');
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('styled text: whole-text edits reach every range, a range edits alone', async ({ session }) => {
  const { page } = session;
  await session.openEditor();
  const send = await connect(session);
  await send([node('Styled text')]);
  await selectLeaf(page);
  const rows = content(page);
  const size = rows.getByRole('spinbutton', { name: 'Size', exact: true }).first();
  await expect(size).toHaveValue('Mixed');
  await expect(rows.locator('[data-run-index]')).toHaveCount(2);

  await typeInto(size, '20');
  expect(await layerData(page, '[layer.d.size, layer.d.runs[0].size.v, layer.d.runs[1].size.v]')).toEqual([20, 20, 20]);
  await undo(page);
  expect(await layerData(page, '[layer.d.runs[0].size.v ?? layer.d.runs[0].size, layer.d.runs[1].size.v ?? layer.d.runs[1].size]')).toEqual([18, 24]);

  const before = await frame(page);
  await rows.locator('[data-run-index="1"] .run-head').click();
  const run = rows.locator('[data-run-index="1"]');
  await page.evaluate(() => { (window as any).__commands.length = 0; });
  await typeInto(run.getByRole('spinbutton', { name: 'Size', exact: true }), '30');
  expect(await commands(page)).toEqual(['set_property c.runs.1.size']);
  expect(await layerData(page, '[layer.d.runs[0].size, layer.d.runs[1].size.v]')).toEqual([18, 30]);
  expect(await frame(page)).not.toBe(before);
  await undo(page);
  expect(await frame(page)).toBe(before);

  await run.getByRole('button', { name: 'Add keyframe for Color' }).click();
  expect(await layerData(page, 'layer.d.runs[1].color.kf.length')).toBe(1);
  expect(await layerData(page, 'typeof layer.d.runs[0].color')).toBe('string');
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('shared edits across a selection show Mixed and apply to each layer', async ({ session }) => {
  const { page } = session;
  await session.openEditor();
  const send = await connect(session);
  const radial = node('RADIAL gradient', 170);
  radial.fills[0]!.gradientStops = [
    { position: 0, color: { r: 1, g: 1, b: 0, a: 1 } },
    { position: 1, color: { r: 0, g: 0, b: 0, a: 1 } },
  ];
  await send([node('LINEAR gradient', 20), radial]);
  await page.evaluate(() => {
    const P = (window as any).PM;
    P.selectLayers(P.proj.layers.map((l: any) => l.id));
    P.bus.emit('layers');
  });
  const rows = content(page);
  await expect(rows.locator('[data-paint="fill"] .pm-select-label')).toHaveText('Mixed');
  await typeInto(rows.getByRole('spinbutton', { name: 'Opacity', exact: true }), '40');
  expect(await page.evaluate(() => (window as any).PM.proj.layers.map((l: any) => l.d.fillGradient.opacity.v))).toEqual([0.4, 0.4]);
  await undo(page);
  expect(await page.evaluate(() => (window as any).PM.proj.layers.map((l: any) => l.d.fillGradient.opacity.v ?? l.d.fillGradient.opacity))).toEqual([1, 1]);
  // A structural edit builds from each layer's own gradient: no stops are copied across.
  await rows.getByRole('button', { name: 'Add fill gradient stop' }).click();
  const stopsOf = () => page.evaluate(() => Object.fromEntries((window as any).PM.proj.layers.map((l: any) => [l.name, l.d.fillGradient.stops.map((s: any) => s.color)])));
  const stops = await stopsOf();
  expect(stops['LINEAR gradient']).toHaveLength(4);
  expect(stops['RADIAL gradient']).toEqual(['#FFFF00FF', '#000000FF', '#808000FF']);
  await undo(page);
  expect(Object.values(await stopsOf()).map((list: any) => list.length).sort()).toEqual([2, 3]);
});

test('import summary names missing fonts; extra mattes and group isolation are editable', async ({ session }) => {
  const { page } = session;
  await session.openEditor();
  const send = await connect(session);
  const text = node('Styled text');
  text.text!.runs[0]!.font = 'Inter Display';
  text.text!.runs[1]!.font = 'Brand Sans Rounded';
  const result = await send([text]);
  expect(result.warnings.map((w: any) => w.reason)).toEqual(['Missing font: Inter Display', 'Missing font: Brand Sans Rounded']);
  const toast = page.locator('#toasts .toast').filter({ hasText: 'missing:' });
  await expect(toast).toContainText('Imported 1 layer. 2 fonts missing: Inter Display, Brand Sans Rounded. Text stays editable with a substitute font.');
  await page.evaluate(() => (window as any).PM.selectLayers([]));
  await toast.getByRole('button', { name: 'Show Layer' }).click();
  expect(await page.evaluate(() => (window as any).PM.sel.layers.map((id: string) => (window as any).PM.L(id).name))).toEqual(['Styled text']);

  await send([node('Luminance mask')]);
  await page.evaluate(() => {
    const P = (window as any).PM;
    const extra = P.mkLayer('shape');
    extra.name = 'Second matte';
    P.proj.layers.splice(0, 0, extra);
    P.touch();
  });
  await selectLeaf(page, 'Masked blue');
  const effects = page.locator('#panel-inspector [data-effects-section]');
  await effects.getByRole('button', { name: 'Add Matte' }).click();
  expect(await layerData(page, 'layer.matteSources.map(m => P.L(m.source).name)')).toEqual(['Second matte']);
  await expect(effects.getByRole('group', { name: 'Track Matte 2 effect' })).toBeVisible();
  // Its mode is a channel: Luma goes through set_property.
  await effects.locator('.fx-params').last().getByRole('combobox', { name: 'Use', exact: true }).click();
  await page.getByRole('option', { name: 'Luma', exact: true }).click();
  expect(await commands(page)).toContain('set_property l.matteSources.0.mode');
  expect(await layerData(page, 'layer.matteSources[0].mode.v')).toBe('luma');
  await undo(page);
  await undo(page);
  expect(await layerData(page, 'layer.matteSources ?? []')).toEqual([]);

  await page.evaluate(() => {
    const P = (window as any).PM;
    P.selectLayers([P.proj.layers.find((l: any) => l.type === 'group').id]);
    P.bus.emit('layers');
  });
  const isolate = page.locator('#panel-inspector .group-rows');
  await isolate.getByRole('radiogroup', { name: 'Isolate' }).getByRole('radio', { name: 'Off' }).click();
  expect(await layerData(page, 'layer.d.isolate')).toBe(false);
  await undo(page);
  expect(await layerData(page, 'layer.d.isolate')).toBe(true);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
