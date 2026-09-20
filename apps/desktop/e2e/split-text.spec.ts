import { chooseNativeMenu, expect, test } from './helpers/app';

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
  await chooseNativeMenu(session, ['Split text into layers…', label], () => page.mouse.click(point.x, point.y, { button: 'right' }));
  const result = await page.evaluate(() => {
    const PM = (window as any).PM;
    const group = PM.firstSel(), members = PM.proj.layers.filter((layer: any) => layer.group === group.id);
    const source = members.at(-1), split = members.filter((layer: any) => layer !== source);
    return { texts: split.map((l: any) => l.d.text),
      positioned: split.every((l: any, i: number) => {
        const mode = split.length === 4 ? 'words' : split.length === 2 ? 'lines' : 'characters';
        const piece = PM.textLayout(source)[mode][i];
        return Math.abs(l.p['anchor.x'].v + piece.x) < 0.001 && Math.abs(l.p['anchor.y'].v + piece.y) < 0.001;
      }),
      editable: split.every((l: any) => l.type === 'text' && l.p.rotation.v === 12),
      grouped: group.type === 'group' && group.name === source.name && members.length === split.length + 1,
      sourceHidden: source.on.v === false };
  });
  expect(result.texts).toEqual(expected); expect(result.editable).toBe(true); expect(result.positioned).toBe(true); expect(result.grouped).toBe(true); expect(result.sourceHidden).toBe(true);
  await page.screenshot({ path: testInfo.outputPath('split-text.png') });
  await page.evaluate(() => (window as any).PM.hist.undo());
  expect(await page.evaluate(() => JSON.stringify((window as any).PM.proj.layers))).toBe(original);
  await page.evaluate(() => (window as any).PM.hist.redo());
  expect(await page.evaluate(() => {
    const PM = (window as any).PM, group = PM.proj.layers.find((layer: any) => layer.type === 'group');
    return PM.proj.layers.filter((layer: any) => layer.group === group.id && layer.on.v !== false).map((layer: any) => layer.d.text);
  })).toEqual(expected);
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

  });
  await chooseNativeMenu(session, ['Split text into layers…', 'By line'], () => page.evaluate(() => { const PM = (window as any).PM; PM.showLayerMenu(PM.proj.layers[0], {clientX: 300, clientY: 200}, 'timeline'); }));
  expect(await page.evaluate(() => {
    const PM = (window as any).PM;
    const group = PM.firstSel();
    return PM.proj.layers.filter((layer: any) => layer.group === group.id && layer.on.v !== false).map((layer: any) => layer.d.text);
  })).toEqual(await page.evaluate(() => (window as any).expectedLines));
  await session.app.evaluate(({ Menu }) => {
    (globalThis as any).__splitPopup = Menu.prototype.popup;
    Menu.prototype.popup = function (options) {
      Menu.prototype.popup = (globalThis as any).__splitPopup;
      (globalThis as any).__splitDisabled = this.items.find(item => item.label === 'Split text into layers…')?.enabled === false;
      options?.callback?.();
    };
  });
  await page.evaluate(() => {
    const PM = (window as any).PM; PM.hist.undo();
    const layer = PM.proj.layers[0]; layer.lock = true;
    PM.showLayerMenu(layer, {clientX: 300, clientY: 200});
  });
  await expect.poll(() => session.app.evaluate(() => (globalThis as any).__splitDisabled)).toBe(true);
  const before = await page.evaluate(() => {
    const PM = (window as any).PM, layer = PM.proj.layers[0]; layer.lock = false; layer.d.text = '';
    return JSON.stringify(PM.proj);
  });
  await chooseNativeMenu(session, ['Split text into layers…', 'By character'], () => page.evaluate(() => { const PM = (window as any).PM; PM.showLayerMenu(PM.proj.layers[0], {clientX: 300, clientY: 200}); }));
  expect(await page.evaluate(() => JSON.stringify((window as any).PM.proj))).toBe(before);
});
