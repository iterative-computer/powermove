import { expect, test } from './helpers/app';

test('edits existing canvas text with native selection, multiline input, cancel and one-step undo', async ({ session }, testInfo) => {
  const { page } = session;
  const id = await page.evaluate(async () => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ name: 'Canvas text editing', w: 800, h: 450, dur: 4, bg: '#202020' });
    const layer = PM.mkLayer('text', {
      d: { text: 'ffagarrag', font: 'Geist', weight: 400, size: 80, color: '#FFFFFF', align: 'center' },
      p: { 'position.x': 400, 'position.y': 180 },
    }, project);
    project.layers = [layer]; PM.replaceProject(project); PM.selectLayers(layer.id);
    PM.setTool('select'); PM.Viewer.fit = true; PM.Viewer.layout(); PM.hist.clear(); PM.invalidate();
    await document.fonts.ready;
    return layer.id;
  });
  const open = async () => {
    const point = await page.evaluate(id => {
      const PM = (window as any).PM, b = PM.Viewer.worldBounds(PM.L(id), PM.time);
      const r = document.querySelector('#stage-inner')!.getBoundingClientRect();
      return { x: r.x + b.cx * PM.Viewer.shown, y: r.y + b.cy * PM.Viewer.shown };
    }, id);
    await page.mouse.dblclick(point.x, point.y);
  };
  const editor = page.getByRole('textbox', { name: 'Edit text on canvas' });
  const source = () => page.evaluate(id => { const d = (window as any).PM.L(id).d; return d.text?.v ?? d.text; }, id);
  await open(); await expect(editor).toBeFocused();
  expect(await page.evaluate(() => window.getSelection()?.toString())).toBe('ffagarrag');
  await page.keyboard.press('ArrowRight');
  for (let i = 0; i < 5; i++) await page.keyboard.press('Shift+ArrowLeft');
  expect(await page.evaluate(() => window.getSelection()?.toString())).toBe('arrag');
  await page.screenshot({ path: testInfo.outputPath('canvas-selection.png') });
  await page.keyboard.press('Meta+C');
  expect(await session.app.evaluate(({ clipboard }) => clipboard.readText())).toBe('arrag');
  await session.app.evaluate(({ clipboard }) => clipboard.writeText(' works'));
  await page.keyboard.press('Meta+V');
  await page.keyboard.press('Enter');
  await page.keyboard.type('Second line');
  expect(await source()).toBe('ffag works\nSecond line');
  expect(await page.evaluate(() => (window as any).PM.Viewer.temporaryTool)).not.toBe('hand');
  await page.keyboard.press('Meta+Enter'); await expect(editor).toHaveCount(0);
  await page.evaluate(() => (window as any).PM.hist.undo()); expect(await source()).toBe('ffagarrag');
  await page.evaluate(() => (window as any).PM.hist.redo()); expect(await source()).toBe('ffag works\nSecond line');
  await open(); await page.keyboard.insertText('Discard'); await page.keyboard.press('Escape');
  expect(await source()).toBe('ffag works\nSecond line');
  expect(session.diagnostics.pageErrors).toEqual([]);
});
