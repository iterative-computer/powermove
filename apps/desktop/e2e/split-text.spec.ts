import { expect, test } from './helpers/app';

for (const [label, expected] of [
  ['By word', ['Hello', 'world', 'Hello', '👨‍👩‍👧‍👦!']],
  ['By character', ['H','e','l','l','o','w','o','r','l','d','H','e','l','l','o','👨‍👩‍👧‍👦','!']],
  ['By line', ['Hello world', 'Hello 👨‍👩‍👧‍👦!']],
] as const) test(`split text ${label} through the layer menu, with undo and redo`, async ({ session }, testInfo) => {
  await session.openEditor();
  const { page } = session;
  await page.waitForFunction(() => { const PM = (window as any).PM; const viewer = PM.Kernel.services.get('viewer'); return Boolean(viewer); });
  const original = await page.evaluate(() => {
    const PM = (window as any).PM;
    const viewer = PM.Kernel.services.get('viewer');
    const project = PM.mkProject({ name: 'Split text', w: 1000, h: 600, dur: 5 });
    const layer = PM.mkLayer('text', { d: { text: 'Hello world\nHello 👨‍👩‍👧‍👦!', size: 50, align: 'center' },
      p: { 'position.x': 450, 'position.y': 200, rotation: 12, 'scale.x': 120 } }, project);
    project.layers = [layer]; PM.replaceProject(project); PM.selectLayers(layer.id);
    PM.hist.clear(); viewer.fit = true; viewer.layout();

    return JSON.stringify(PM.proj.layers);
  });
  const point = await page.evaluate(() => {
    const PM = (window as any).PM;
    const viewer = PM.Kernel.services.get('viewer');
    const layer = PM.proj.layers[0];
    const glyph = PM.textLayout(layer).characters[0], m = PM.worldMatrix(layer, PM.time);
    const x = glyph.x + 15, y = glyph.y + 20, rect = document.querySelector('#stage-inner')!.getBoundingClientRect();
    return { x: rect.x + (m[0]*x + m[2]*y + m[4])*viewer.shown, y: rect.y + (m[1]*x + m[3]*y + m[5])*viewer.shown };
  });
  await page.mouse.click(point.x, point.y, { button: 'right' });
  await page.getByText('Split text into layers…', { exact: true }).click();
  await page.getByText(label, { exact: true }).click();
  const result = await page.evaluate(() => {
    const PM = (window as any).PM;
    return { texts: PM.selLayers().map((l: any) => l.d.text),
      positioned: PM.selLayers().every((l: any, i: number) => {
        const source = PM.proj.layers.at(-1), mode = PM.selLayers().length === 4 ? 'words' : PM.selLayers().length === 2 ? 'lines' : 'characters';
        const piece = PM.textLayout(source)[mode][i];
        return Math.abs(l.p['anchor.x'].v + piece.x) < 0.001 && Math.abs(l.p['anchor.y'].v + piece.y) < 0.001;
      }),
      editable: PM.selLayers().every((l: any) => l.type === 'text' && l.p.rotation.v === 12),
      sourceHidden: PM.proj.layers.at(-1).on.v === false };
  });
  expect(result.texts).toEqual(expected); expect(result.editable).toBe(true); expect(result.positioned).toBe(true); expect(result.sourceHidden).toBe(true);
  await page.screenshot({ path: testInfo.outputPath('split-text.png') });
  await page.evaluate(() => (window as any).PM.hist.undo());
  expect(await page.evaluate(() => JSON.stringify((window as any).PM.proj.layers))).toBe(original);
  await page.evaluate(() => (window as any).PM.hist.redo());
  expect(await page.evaluate(() => (window as any).PM.proj.layers.slice(0, -1).map((l: any) => l.d.text))).toEqual(expected);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('wrapped lines, locked layers and blank text', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await page.waitForFunction(() => { const PM = (window as any).PM; const viewer = PM.Kernel.services.get('viewer'); return Boolean(viewer); });
  await page.evaluate(() => {
    const PM = (window as any).PM, project = PM.mkProject({ w: 800, h: 600, dur: 5 });
    const layer = PM.mkLayer('text', { d: { text: 'One two three four five', size: 40, paragraph: true, boxWidth: 130, boxHeight: 400 } }, project);
    project.layers = [layer]; PM.replaceProject(project); PM.selectLayers(layer.id); PM.hist.clear();
    (window as any).expectedLines = PM.textLayout(layer).lines.map((p: any) => p.text);
    PM.showLayerMenu(layer, {clientX: 300, clientY: 200}, 'timeline');
  });
  await page.getByText('Split text into layers…', { exact: true }).click();
  await page.getByText('By line', { exact: true }).click();
  expect(await page.evaluate(() => {
    const PM = (window as any).PM;
    return PM.selLayers().map((l: any) => l.d.text);
  })).toEqual(await page.evaluate(() => (window as any).expectedLines));
  await page.evaluate(() => {
    const PM = (window as any).PM; PM.hist.undo();
    const layer = PM.proj.layers[0]; layer.lock = true;
    PM.showLayerMenu(layer, {clientX: 300, clientY: 200});
  });
  await expect(page.locator('.di').filter({ hasText: 'Split text into layers…' })).toHaveAttribute('aria-disabled', 'true');
  await page.keyboard.press('Escape');
  const before = await page.evaluate(() => {
    const PM = (window as any).PM, layer = PM.proj.layers[0]; layer.lock = false; layer.d.text = '';
    PM.showLayerMenu(layer, {clientX: 300, clientY: 200}); return JSON.stringify(PM.proj);
  });
  await page.getByText('Split text into layers…', { exact: true }).click();
  await page.getByText('By character', { exact: true }).click();
  expect(await page.evaluate(() => JSON.stringify((window as any).PM.proj))).toBe(before);
});
