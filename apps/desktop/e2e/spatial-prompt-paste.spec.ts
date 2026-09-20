import { test, expect } from './helpers/app';

for (const surface of ['floating', 'panel'] as const) test(`${surface} prompt pastes native text and clipboard attachments without changing layers`, async ({ session }) => {
  await session.openEditor();
  const { page, app } = session;
  await page.evaluate((surface) => {
    const PM = (window as any).PM;
    PM.WindowCapture.request = async () => null;
    if (surface === 'floating') PM.SpatialAssistant.activate(400, 300);
    else PM.SpatialAssistant.open();
  }, surface);
  const composer = page.locator(surface === 'floating' ? '.spatial-compose' : '.agent-composer');
  const input = surface === 'floating' ? composer.locator('textarea') : composer.getByRole('textbox');
  const value = () => input.evaluate((el: any) => el.value ?? (window as any).PM.AgentUI.state.composerDraft);
  await expect(input).toBeVisible();
  const layers = await page.evaluate(() => (window as any).PM.proj.layers.length);
  const originalClipboard = await app.evaluateHandle(async ({ clipboard, ClipboardItem }) => Promise.all((await clipboard.read()).map(async item => new ClipboardItem(Object.fromEntries(await Promise.all(item.types.map(async type => [type, await item.getType(type)])))))));
  await app.evaluate(({ clipboard }) => clipboard.writeText('Pasted text\nSecond line'));
  await app.evaluate(({ BrowserWindow }) => {
    const contents = BrowserWindow.getAllWindows()[0].webContents as any;
    const paste = contents.paste.bind(contents);
    contents.__spatialPasteCount = 0;
    contents.paste = () => { contents.__spatialPasteCount += 1; paste(); };
  });
  try {
    await input.focus();
    await page.keyboard.press('Meta+V');
    await expect.poll(value).toBe('Pasted text\nSecond line');
    expect(await app.evaluate(({ BrowserWindow }) => (BrowserWindow.getAllWindows()[0].webContents as any).__spatialPasteCount)).toBe(1);
    await input.selectText();
    await page.keyboard.press('Control+V');
    await expect.poll(value).toBe('Pasted text\nSecond line');
    expect(await app.evaluate(({ BrowserWindow }) => (BrowserWindow.getAllWindows()[0].webContents as any).__spatialPasteCount)).toBe(2);
    const imageBytes = [...await page.screenshot()];
    await app.evaluate(async ({ clipboard, ClipboardItem }, data) => {
      const bytes = Uint8Array.from(data);
      await clipboard.write([new ClipboardItem({ 'image/png': new Blob([bytes], { type: 'image/png' }) })]);
    }, imageBytes);
    await input.focus();
    await page.keyboard.press('Meta+V');
    await expect(composer.locator(surface === 'floating' ? '.agent-attachment img' : '.agent-inline-attachment img')).toHaveCount(1);
    await expect.poll(value).toBe('Pasted text\nSecond line');
    expect(await page.evaluate(() => (window as any).PM.proj.layers.length)).toBe(layers);
  } finally {
    await app.evaluate(({ clipboard }, items) => clipboard.write(items), originalClipboard);
    await originalClipboard.dispose();
  }
});
