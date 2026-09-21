import { expect, test } from './helpers/app';

test('media action buttons retain native Enter and Space activation', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await page.evaluate(async () => {
    const canvas = document.createElement('canvas');
    canvas.width = canvas.height = 16;
    const blob = await new Promise<Blob>(resolve => canvas.toBlob(value => resolve(value!), 'image/png'));
    await (window as any).PM.importFiles([new File([blob], 'Keyboard.png', { type: 'image/png' })], { placement: null });
  });
  const card = page.locator('.asset-card').filter({ hasText: 'Keyboard.png' });
  await expect(card).toBeVisible();
  await card.getByRole('button', { name: 'Add Keyboard.png to timeline', exact: true }).focus();
  await page.keyboard.press('Space');
  await expect.poll(() => page.evaluate(() => (window as any).PM.proj.layers.filter((layer: any) => layer.name === 'Keyboard.png').length)).toBe(1);

  await session.app.evaluate(({ dialog }) => {
    dialog.showMessageBox = async (_window, options) => {
      (globalThis as any).__keyboardConfirmation = options.message;
      return { response: 0, checkboxChecked: false };
    };
  });
  await card.getByRole('button', { name: 'Delete Keyboard.png', exact: true }).focus();
  await page.keyboard.press('Enter');
  await expect.poll(() => session.app.evaluate(() => (globalThis as any).__keyboardConfirmation)).toBe('Delete “Keyboard.png”?');
  await expect(card).toHaveCount(0);
  expect(await page.evaluate(() => (window as any).PM.proj.layers.filter((layer: any) => layer.name === 'Keyboard.png').length)).toBe(0);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
