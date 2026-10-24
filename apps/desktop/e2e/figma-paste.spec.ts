import { expect, test } from './helpers/app';
import { figmaHtml, figmaNode, guid, pathBlob } from '../src/renderer/src/legacy/core/figma/__fixtures__/clipboard';

test.beforeEach(async ({ session }) => {
  await session.openEditor();
  await session.app.evaluate(({ BrowserWindow }) => { BrowserWindow.getAllWindows()[0].isFocused = () => true; });
  await session.page.evaluate(() => {
    const PM = (window as any).PM;
    PM.replaceProject(PM.mkProject({ name: 'Figma paste', w: 640, h: 360, dur: 8, bg: '#000000' }));
    PM.setTime(2); PM.hist.clear(); (document.activeElement as HTMLElement)?.blur();
  });
});

test('native copy and paste creates editable grouped layers, preserves rendered placement and undoes once', async ({ session }) => {
  const { app, page } = session;
  const saved = await app.evaluateHandle(async ({ clipboard, ClipboardItem }) => Promise.all((await clipboard.read()).map(async item => new ClipboardItem(Object.fromEntries(await Promise.all(item.types.map(async type => [type, await item.getType(type)])))))));
  try {
    const html = await figmaHtml([
      figmaNode(1, 'GROUP', { name: 'Figma artwork', fillPaints: [], size: { x: 200, y: 100 }, transform: { m00: 1, m01: 0, m02: 1000, m10: 0, m11: 1, m12: 500 } }),
      figmaNode(2, 'RECTANGLE', { name: 'Orange card', size: { x: 200, y: 100 }, parentIndex: { guid: guid(1), position: '!' } }),
      figmaNode(3, 'VECTOR', { name: 'Blue triangle', parentIndex: { guid: guid(1), position: '"' }, fillGeometry: [{ commandsBlob: 0 }],
        fillPaints: [{ type: 'SOLID', color: { r: 0, g: 0, b: 1, a: 1 } }] }),
      figmaNode(4, 'TEXT', { name: 'Editable title', parentIndex: { guid: guid(1), position: '#' }, size: { x: 80, y: 20 },
        fontName: { family: 'Arial', style: 'Regular', postscript: 'ArialMT' }, fontSize: 16, textAutoResize: 'WIDTH_AND_HEIGHT', textData: { characters: 'Hello Figma' },
        transform: { m00: 1, m01: 0, m02: 110, m10: 0, m11: 1, m12: 10 }, fillPaints: [{ type: 'SOLID', color: { r: 1, g: 1, b: 1, a: 1 } }] }),
    ], [pathBlob()], 'zstd');
    await app.evaluate(async ({ clipboard, ClipboardItem }, html) => { await clipboard.write([new ClipboardItem({ 'text/html': html })]); }, html);
    await page.keyboard.press('Meta+V');
    await expect.poll(() => page.evaluate(() => (window as any).PM.proj.layers.map((l: any) => l.name)))
      .toEqual(['Figma artwork', 'Editable title', 'Blue triangle', 'Orange card']);
    const proof = await page.evaluate(() => {
      const PM = (window as any).PM;
      const canvas = PM.renderFrameTo(2, 640, 360), ctx = canvas.getContext('2d');
      const pixel = (x: number, y: number) => [...ctx.getImageData(x, y, 1, 1).data];
      return { pixels: [pixel(225, 135), pixel(400, 210), pixel(210, 125)],
        types: PM.proj.layers.map((l: any) => l.type), times: PM.proj.layers.map((l: any) => l.from),
        text: PM.proj.layers[1].d.text, vertices: PM.proj.layers[2].d.paths[0].vertices.length };
    });
    expect(proof.types).toEqual(['group', 'text', 'shape', 'shape']); expect(proof.times).toEqual([2, 2, 2, 2]);
    expect(proof.text).toBe('Hello Figma'); expect(proof.vertices).toBe(3);
    expect(proof.pixels[0]).toEqual([0, 0, 255, 255]);
    expect(proof.pixels[1]).toEqual([255, 64, 0, 255]);
    expect(proof.pixels[2]).toEqual([0, 0, 0, 255]);
    await page.keyboard.press('Meta+Z');
    await expect.poll(() => page.evaluate(() => (window as any).PM.proj.layers.length)).toBe(0);
    await page.keyboard.press('Meta+Shift+Z');
    await expect.poll(() => page.evaluate(() => (window as any).PM.proj.layers.length)).toBe(4);
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally { await app.evaluate(({ clipboard }, items) => clipboard.write(items), saved); await saved.dispose(); }
});

for (const proceed of [false, true]) test(`unsupported Figma paste ${proceed ? 'uses fallback after informing the user' : 'can be cancelled before anything is imported'}`, async ({ session }) => {
  const { app, page } = session;
  const saved = await app.evaluateHandle(async ({ clipboard, ClipboardItem }) => Promise.all((await clipboard.read()).map(async item => new ClipboardItem(Object.fromEntries(await Promise.all(item.types.map(async type => [type, await item.getType(type)])))))));
  try {
    const html = await figmaHtml([figmaNode(1, 'RECTANGLE', { name: 'Shadow card', effects: [{ type: 'DROP_SHADOW', visible: true }] })]);
    const png = await page.evaluate(async () => {
      const canvas = document.createElement('canvas'); canvas.width = 200; canvas.height = 120;
      const ctx = canvas.getContext('2d')!; ctx.fillStyle = '#ff4000'; ctx.fillRect(0, 0, 200, 120);
      const blob = await new Promise<Blob>(resolve => canvas.toBlob(blob => resolve(blob!), 'image/png'));
      return [...new Uint8Array(await blob.arrayBuffer())];
    });
    await app.evaluate(({ dialog }, proceed) => {
      (globalThis as any).figmaNotices = [];
      dialog.showMessageBox = async (_window, options) => {
        (globalThis as any).figmaNotices.push(options);
        return { response: proceed ? 0 : 1, checkboxChecked: false };
      };
    }, proceed);
    await app.evaluate(async ({ clipboard, ClipboardItem }, { html, png }) => {
      await clipboard.write([new ClipboardItem({ 'text/html': html, 'image/png': new Blob([Uint8Array.from(png)], { type: 'image/png' }) })]);
    }, { html, png });
    await page.keyboard.press('Meta+V');
    await expect.poll(() => app.evaluate(() => (globalThis as any).figmaNotices.length)).toBe(1);
    const notice = await app.evaluate(() => (globalThis as any).figmaNotices[0]);
    expect(notice.detail).toContain('Shadow card'); expect(notice.detail).toContain('effects');
    expect(notice.detail).toContain('individual parts will not be editable'); expect(notice.buttons).toContain('Paste as Image');
    if (proceed) {
      await expect.poll(() => page.evaluate(() => (window as any).PM.proj.layers.map((l: any) => l.type))).toEqual(['image']);
      expect(await page.evaluate(() => { const l = (window as any).PM.proj.layers[0]; return { from: l.from, w: l.d.w, h: l.d.h }; })).toEqual({ from: 2, w: 100, h: 60 });
      await page.keyboard.press('Meta+Z');
      await expect.poll(() => page.evaluate(() => (window as any).PM.proj.layers.length)).toBe(0);
    } else {
      expect(await page.evaluate(() => ({ layers: (window as any).PM.proj.layers.length, assets: Object.keys((window as any).PM.proj.assets).length }))).toEqual({ layers: 0, assets: 0 });
    }
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally { await app.evaluate(({ clipboard }, items) => clipboard.write(items), saved); await saved.dispose(); }
});
