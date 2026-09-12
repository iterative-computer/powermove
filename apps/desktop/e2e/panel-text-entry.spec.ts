import { expect, test } from './helpers/app';

test('Command+V pastes text into a panel search field without pasting layers', async ({ session }) => {
  const { page } = session;
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.replaceProject(PM.mkProject({ name: 'Panel text paste QA', dur: 10 }));
    const layer = PM.mkLayer('solid', { name: 'Paste target' });
    PM.proj.layers.push(layer);
    PM.selectLayers(layer.id);
    PM.hist.clear();
    PM.invalidate();
  });
  const effectsPanel = page.locator('.panel[data-panel="fxbrowser"]');
  await expect(effectsPanel).toBeVisible();
  await effectsPanel.getByRole('button', { name: 'Search', exact: true }).click();
  const field = effectsPanel.getByRole('searchbox', { name: 'Search effects', exact: true });
  await field.fill('Before ');
  await session.app.evaluate(({ clipboard }) => clipboard.writeText('pasted text'));
  await session.app.evaluate(({ BrowserWindow }) => {
    const contents = BrowserWindow.getAllWindows()[0]?.webContents as any;
    if (!contents) throw new Error('Hidden editor window is unavailable');
    const nativePaste = contents.paste.bind(contents);
    (globalThis as any).__panelNativePasteCalls = 0;
    contents.paste = () => {
      (globalThis as any).__panelNativePasteCalls += 1;
      nativePaste();
    };
  });

  await field.focus();
  await field.evaluate((input: HTMLInputElement) => {
    input.setSelectionRange(input.value.length, input.value.length);
  });
  await page.keyboard.press('Meta+V');

  await expect(field).toHaveValue('Before pasted text');
  expect(await session.app.evaluate(() => (globalThis as any).__panelNativePasteCalls)).toBe(1);
  expect(await page.evaluate(() => (window as any).PM.proj.layers.length)).toBe(1);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
