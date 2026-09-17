import { expect, test } from './helpers/app';

/* On-canvas text editing contract. The compositor paints the text while it
   is edited; a hidden textarea owns input. Escape, Command+Enter and clicking
   away all commit. Undo reverts a whole session in one step. */


async function setup(session: any) {
  await session.openEditor();
  const { page } = session;
  const id = await page.evaluate(async () => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ name: 'Canvas text editing', w: 800, h: 450, dur: 4, bg: '#202020' });
    const layer = PM.mkLayer('text', {
      d: { text: 'ffagarrag', font: 'Geist', weight: 400, size: 80, color: '#FFFFFF', align: 'center' },
      p: { 'position.x': 400, 'position.y': 180 },
    }, project);
    project.layers = [layer]; PM.replaceProject(project); PM.selectLayers(layer.id);
    const tool = PM.Kernel.services.get('tool');
    const viewer = PM.Kernel.services.get('viewer');
    tool.setTool('select'); viewer.fit = true; viewer.layout(); PM.hist.clear(); PM.invalidate();
    await document.fonts.ready;
    return layer.id;
  });
  const stagePoint = (x: number, y: number) => page.evaluate(([x, y]: number[]) => {
    const PM = (window as any).PM, viewer = PM.Kernel.services.get('viewer');
    const r = document.querySelector('#stage-inner')!.getBoundingClientRect();
    return { x: r.x + x * viewer.shown, y: r.y + y * viewer.shown };
  }, [x, y]);
  const layerCenter = (layerId: string) => page.evaluate(layerId => {
    const PM = (window as any).PM, viewer = PM.Kernel.services.get('viewer'), b = viewer.worldBounds(PM.L(layerId), PM.time);
    const r = document.querySelector('#stage-inner')!.getBoundingClientRect();
    return { x: r.x + b.cx * viewer.shown, y: r.y + b.cy * viewer.shown };
  }, layerId);
  const source = (layerId = id) => page.evaluate(layerId => { const d = (window as any).PM.L(layerId)?.d; return d ? (d.text?.v ?? d.text) : null; }, layerId);
  const state = () => page.evaluate(() => {
    const PM = (window as any).PM, viewer = PM.Kernel.services.get('viewer'), tool = PM.Kernel.services.get('tool');
    return { editing: viewer.canvasTextEditing ?? null, selection: viewer.textSelection ?? null, tool: tool.tool, layers: PM.proj.layers.map((l: any) => ({ id: l.id, name: l.name, type: l.type, boxWidth: l.d.boxWidth?.v ?? l.d.boxWidth, boxHeight: l.d.boxHeight?.v ?? l.d.boxHeight })), selected: PM.sel.layers ?? PM.sel };
  });
  const editor = page.getByRole('textbox', { name: 'Edit text on canvas' });
  return { page, id, stagePoint, layerCenter, source, state, editor };
}

test('edits existing text in place: caret, selection, clipboard, multiline, commit and one-step undo', async ({ session }, testInfo) => {
  const { page, id, layerCenter, source, state, editor } = await setup(session);
  const center = await layerCenter(id);
  await page.mouse.dblclick(center.x, center.y);
  await expect(editor).toBeFocused();
  expect((await state()).selection).toEqual({ layer: id, start: 0, end: 9 });
  // The layer keeps rendering while it is edited; nothing is hidden.
  expect((await state()).editing).toBe(id);
  await page.keyboard.press('ArrowRight');
  for (let i = 0; i < 5; i++) await page.keyboard.press('Shift+ArrowLeft');
  expect((await state()).selection).toEqual({ layer: id, start: 4, end: 9 });
  await page.screenshot({ path: testInfo.outputPath('canvas-selection.png') });
  await page.keyboard.press('Meta+C');
  expect(await session.app.evaluate(({ clipboard }) => clipboard.readText())).toBe('arrag');
  await session.app.evaluate(({ clipboard }) => clipboard.writeText(' works'));
  await page.keyboard.press('Meta+V');
  await page.keyboard.press('Enter');
  await page.keyboard.type('Second line');
  expect(await source()).toBe('ffag works\nSecond line');
  await page.keyboard.press('Meta+Enter');
  await expect(editor).toHaveCount(0);
  expect((await state()).editing).toBeNull();
  await page.evaluate(() => (window as any).PM.hist.undo()); expect(await source()).toBe('ffagarrag');
  await page.evaluate(() => (window as any).PM.hist.redo()); expect(await source()).toBe('ffag works\nSecond line');

  // Escape commits too. Typed text is never thrown away by leaving.
  await page.mouse.dblclick(center.x, center.y);
  await expect(editor).toBeFocused();
  await page.keyboard.insertText('Kept');
  await page.keyboard.press('Escape');
  await expect(editor).toHaveCount(0);
  expect(await source()).toBe('Kept');
  await page.evaluate(() => (window as any).PM.hist.undo());
  expect(await source()).toBe('ffag works\nSecond line');
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('text tool: click creates point text, typing renders live, clicking away commits and returns to Select', async ({ session }) => {
  const { page, id, stagePoint, source, state, editor } = await setup(session);
  await page.evaluate(() => (window as any).PM.Kernel.services.get('tool').setTool('text'));
  const spot = await stagePoint(120, 380);
  await page.mouse.click(spot.x, spot.y);
  await expect(editor).toBeFocused();
  let s = await state();
  expect(s.layers).toHaveLength(2);
  const created = s.layers.find(l => l.id !== id)!;
  expect(created).toMatchObject({ type: 'text', name: 'Text', boxWidth: 0, boxHeight: 0 });
  expect(s.editing).toBe(created.id);
  await page.keyboard.type('Hello there');
  expect(await source(created.id)).toBe('Hello there');
  // Click on empty canvas: commits, returns to Select, does not plant another layer.
  const away = await stagePoint(700, 60);
  await page.mouse.click(away.x, away.y);
  await expect(editor).toHaveCount(0);
  s = await state();
  expect(s.layers).toHaveLength(2);
  expect(s.tool).toBe('select');
  expect(s.layers.find(l => l.id === created.id)!.name).toBe('Hello there');
  expect(await source(created.id)).toBe('Hello there');
  // Creating, typing and naming is one undo step (the click away may have
  // pushed a selection entry on top of it).
  await page.evaluate(() => { const PM = (window as any).PM; for (let i = 0; i < 3 && PM.hist.label() !== 'Add text'; i++) PM.hist.undo(); });
  expect(await page.evaluate(() => (window as any).PM.hist.label())).toBe('Add text');
  await page.evaluate(() => (window as any).PM.hist.undo());
  expect((await state()).layers).toHaveLength(1);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('text tool: leaving a new layer empty removes it without a history entry', async ({ session }) => {
  const { page, id, stagePoint, state, editor } = await setup(session);
  const historyBefore = await page.evaluate(() => (window as any).PM.hist.canUndo?.() ?? null);
  await page.evaluate(() => (window as any).PM.Kernel.services.get('tool').setTool('text'));
  const spot = await stagePoint(120, 380);
  await page.mouse.click(spot.x, spot.y);
  await expect(editor).toBeFocused();
  expect((await state()).layers).toHaveLength(2);
  await page.keyboard.press('Escape');
  await expect(editor).toHaveCount(0);
  const s = await state();
  expect(s.layers.map(l => l.id)).toEqual([id]);
  expect(s.tool).toBe('select');
  expect(await page.evaluate(() => (window as any).PM.hist.canUndo?.() ?? null)).toBe(historyBefore);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('text tool: dragging creates a fixed-width paragraph with automatic height', async ({ session }) => {
  const { page, id, stagePoint, source, state, editor } = await setup(session);
  await page.evaluate(() => (window as any).PM.Kernel.services.get('tool').setTool('text'));
  const from = await stagePoint(80, 300), to = await stagePoint(380, 420);
  await page.mouse.move(from.x, from.y);
  await page.mouse.down();
  await page.mouse.move(to.x, to.y, { steps: 6 });
  await page.mouse.up();
  await expect(editor).toBeFocused();
  const created = (await state()).layers.find(l => l.id !== id)!;
  expect(created.boxWidth).toBeCloseTo(300, 0);
  expect(created.boxHeight).toBe(0);
  await page.keyboard.type('Wrapped paragraph text that runs past the box width and grows downward');
  expect(await source(created.id)).toContain('grows downward');
  await page.keyboard.press('Meta+Enter');
  await expect(editor).toHaveCount(0);
  expect((await state()).layers).toHaveLength(2);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('clicking inside the edited text moves the caret; double-click selects a word', async ({ session }) => {
  const { page, id, layerCenter, state, editor } = await setup(session);
  const center = await layerCenter(id);
  await page.mouse.dblclick(center.x, center.y);
  await expect(editor).toBeFocused();
  const bounds = await page.evaluate(id => {
    const PM = (window as any).PM, viewer = PM.Kernel.services.get('viewer'), b = viewer.worldBounds(PM.L(id), PM.time);
    const r = document.querySelector('#stage-inner')!.getBoundingClientRect();
    return { left: r.x + b.x0 * viewer.shown, right: r.x + b.x1 * viewer.shown, y: r.y + b.cy * viewer.shown };
  }, id);
  await page.mouse.click(bounds.left + 2, bounds.y);
  let s = await state();
  expect(s.editing).toBe(id);
  expect(s.selection).toEqual({ layer: id, start: 0, end: 0 });
  await page.mouse.click(bounds.right - 2, bounds.y);
  expect((await state()).selection).toEqual({ layer: id, start: 9, end: 9 });
  await page.mouse.dblclick(center.x, center.y);
  expect((await state()).selection).toEqual({ layer: id, start: 0, end: 9 });
  await page.keyboard.type('X');
  await page.keyboard.press('Escape');
  expect(await page.evaluate(id => { const d = (window as any).PM.L(id).d; return d.text?.v ?? d.text; }, id)).toBe('X');
  expect(session.diagnostics.pageErrors).toEqual([]);
});
