import { expect, test } from './helpers/app';

test.describe('@menu-save save command routing', () => {
  test('routes both the renderer accelerator and native menu item to PM.cmd', async ({ session }) => {
    const { app, page } = session;
    await page.evaluate(() => {
      const PM = (window as any).PM;
      (window as any).__e2eCommandCalls = [];
      PM.cmd = (id: string) => (window as any).__e2eCommandCalls.push(id);
    });

    await page.locator('body').click({ position: { x: 20, y: 100 } });
    await page.keyboard.press('Meta+S');
    await expect.poll(() => page.evaluate(() => (window as any).__e2eCommandCalls)).toContain('save');

    const menuWired = await app.evaluate(({ Menu }) => {
      const item = Menu.getApplicationMenu()?.getMenuItemById('save');
      if (!item) return false;
      item.click();
      return true;
    });
    expect(menuWired).toBe(true);

    await expect.poll(() => page.evaluate(() => (window as any).__e2eCommandCalls)).toEqual(['save', 'save']);
  });
});
