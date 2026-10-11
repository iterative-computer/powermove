import { expect, test } from './helpers/app';
import { fixturePath } from './helpers/media';
import { pathToFileURL } from 'node:url';

test.beforeEach(async ({ session }) => {
  await session.openEditor();
  // The hidden harness has no OS focus; still require real input in main.
  await session.app.evaluate(({ BrowserWindow }) => {
    BrowserWindow.getAllWindows()[0].isFocused = () => true;
  });
  await session.page.evaluate(() => {
    const PM = (window as any).PM;
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: PM.mkProject({ name: 'Media paste', w: 640, h: 360, dur: 8 }) }));
    PM.setTime(2);
    PM.hist.clear();
    document.activeElement instanceof HTMLElement && document.activeElement.blur();
  });
});

for (const [name, type] of [['tone.wav', 'audio'], ['h264-aac.mp4', 'video']]) {
  test(`pastes a copied ${type} file from Finder`, async ({ session }) => {
    test.skip(process.platform !== 'darwin', 'Finder uses the macOS file clipboard format');
    const { app, page } = session;
    const originalClipboard = await app.evaluateHandle(async ({ clipboard, ClipboardItem }) => Promise.all((await clipboard.read()).map(async item => new ClipboardItem(Object.fromEntries(await Promise.all(item.types.map(async type => [type, await item.getType(type)])))))));
    try {
      await app.evaluate(async ({ clipboard, ClipboardItem }, url) => {
        await clipboard.write([new ClipboardItem({ 'electron application/osclipboard;format="public.file-url"': url })]);
      }, pathToFileURL(fixturePath(name!)).href);
      await page.keyboard.press('Meta+V');
      await expect.poll(() => page.evaluate(() => (window as any).PM.proj.layers.map((layer: any) => layer.type))).toEqual([type]);
      expect(await page.evaluate(() => (window as any).PM.proj.layers[0].from)).toBe(2);
      expect(session.diagnostics.pageErrors).toEqual([]);
    } finally {
      await app.evaluate(({ clipboard }, items) => clipboard.write(items), originalClipboard);
      await originalClipboard.dispose();
    }
  });
}

test('pastes a clipboard image at the playhead and supports undo and redo', async ({ session }) => {
  const { app, page } = session;
  const originalClipboard = await app.evaluateHandle(async ({ clipboard, ClipboardItem }) => Promise.all((await clipboard.read()).map(async item => new ClipboardItem(Object.fromEntries(await Promise.all(item.types.map(async type => [type, await item.getType(type)])))))));
  // Exercise the real OS clipboard and native shortcut, rather than a synthetic paste.
  try {
    const png = await page.evaluate(async () => {
      const canvas = document.createElement('canvas'); canvas.width = 8; canvas.height = 6;
      const ctx = canvas.getContext('2d')!; ctx.fillStyle = '#ff8800'; ctx.fillRect(0, 0, 8, 6);
      const blob = await new Promise<Blob>(resolve => canvas.toBlob(value => resolve(value!), 'image/png'));
      return [...new Uint8Array(await blob.arrayBuffer())];
    });
    await app.evaluate(async ({ clipboard, ClipboardItem }, png) => {
      await clipboard.write([new ClipboardItem({ 'image/png': new Blob([Uint8Array.from(png)], { type: 'image/png' }) })]);
    }, png);
    await page.keyboard.press(process.platform === 'darwin' ? 'Meta+V' : 'Control+V');
    await expect.poll(() => page.evaluate(() => (window as any).PM.proj.layers.length)).toBe(1);
    expect(await page.evaluate(() => {
      const PM = (window as any).PM;
      return { type: PM.proj.layers[0].type, from: PM.proj.layers[0].from, assets: Object.keys(PM.proj.assets).length };
    })).toEqual({ type: 'image', from: 2, assets: 1 });
    await page.keyboard.press(process.platform === 'darwin' ? 'Meta+Z' : 'Control+Z');
    await expect.poll(() => page.evaluate(() => (window as any).PM.proj.layers.length)).toBe(0);
    await page.keyboard.press(process.platform === 'darwin' ? 'Meta+Shift+Z' : 'Control+Shift+Z');
    await expect.poll(() => page.evaluate(() => (window as any).PM.proj.layers.length)).toBe(1);
    expect(session.diagnostics.pageErrors).toEqual([]);
    await page.evaluate(() => {
      const PM = (window as any).PM; PM.selectLayers([PM.proj.layers[0].id]);
    });
    await page.keyboard.press(process.platform === 'darwin' ? 'Meta+C' : 'Control+C');
    await page.keyboard.press(process.platform === 'darwin' ? 'Meta+V' : 'Control+V');
    await expect.poll(() => page.evaluate(() => (window as any).PM.hist.list().at(-1))).toBe('Paste layers');
    expect(await page.evaluate(() => (window as any).PM.proj.layers.length)).toBe(2);
  } finally {
    await app.evaluate(({ clipboard }, items) => clipboard.write(items), originalClipboard);
    await originalClipboard.dispose();
  }
});
