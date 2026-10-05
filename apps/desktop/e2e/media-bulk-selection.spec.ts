import { expect, test } from './helpers/app';

async function importMedia(page: import('@playwright/test').Page, count = 4, timeline = false) {
  await page.evaluate(async ({ count, timeline }) => {
    const canvas = document.createElement('canvas');
    canvas.width = canvas.height = 16;
    const files: File[] = [];
    for (let index = 0; index < count; index++) {
      const context = canvas.getContext('2d')!;
      context.fillStyle = `hsl(${index * 70}, 70%, 50%)`;
      context.fillRect(0, 0, canvas.width, canvas.height);
      const blob = await new Promise<Blob>(resolve => canvas.toBlob(value => resolve(value!), 'image/png'));
      files.push(new File([blob], `Bulk-${String.fromCharCode(65 + index)}.png`, { type: 'image/png' }));
    }
    await (window as any).PM.importFiles(files, timeline ? {} : { placement: null });
  }, { count, timeline });
  await expect(page.locator('.asset-card[data-asset-id]')).toHaveCount(count);
}

test('drag a selection box, delete media, and undo the whole batch in both themes', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  for (const theme of ['light', 'dark']) {
    await page.evaluate(theme => (window as any).PM.theme.apply(theme), theme);
    if (theme === 'light') await importMedia(page);
    const list = page.getByRole('listbox', { name: 'Project media' });
    const cards = page.locator('.asset-card[data-asset-id]');
    const bounds = await list.boundingBox();
    const first = await cards.first().boundingBox();
    const last = await cards.last().boundingBox();
    expect(bounds && first && last).toBeTruthy();
    await page.mouse.move(bounds!.x + 2, first!.y - 2);
    await page.mouse.down();
    await page.mouse.move(bounds!.x + bounds!.width - 2, last!.y + last!.height + 2, { steps: 12 });
    await expect(page.locator('.asset-selection-box')).toBeVisible();
    await expect(page.locator('.asset-card[data-asset-id][aria-selected="true"]')).toHaveCount(4);
    await page.mouse.up();
    await expect(page.getByRole('button', { name: 'Delete selected media', exact: true })).toBeVisible();
    await page.keyboard.press('Backspace');
    await expect(cards).toHaveCount(0);
    await page.evaluate(() => (window as any).PM.hist.undo());
    await expect(cards).toHaveCount(4);
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('Command-click and Shift-click select media; referenced bulk deletion confirms once', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await importMedia(page, 4, true);
  const cards = page.locator('.asset-card[data-asset-id]');
  await cards.nth(0).locator('.asset-copy').click();
  await cards.nth(2).locator('.asset-copy').click({ modifiers: ['Meta'] });
  await expect(page.locator('.asset-card[data-asset-id][aria-selected="true"]')).toHaveCount(2);
  await cards.nth(3).locator('.asset-copy').click({ modifiers: ['Shift'] });
  await expect(page.locator('.asset-card[data-asset-id][aria-selected="true"]')).toHaveCount(2);
  await page.keyboard.press('Meta+a');
  await expect(page.locator('.asset-card[data-asset-id][aria-selected="true"]')).toHaveCount(4);
  await session.app.evaluate(({ dialog }) => {
    dialog.showMessageBox = async (_window, options) => {
      (globalThis as any).__bulkConfirmation = options.message;
      return { response: 0, checkboxChecked: false };
    };
  });
  await page.getByRole('button', { name: 'Delete selected media', exact: true }).focus();
  await page.keyboard.press('Space');
  await expect.poll(() => session.app.evaluate(() => (globalThis as any).__bulkConfirmation)).toBe('Delete 4 media files?');
  await expect(cards).toHaveCount(0);
  expect(await page.evaluate(() => (window as any).PM.proj.layers.length)).toBe(0);
  await page.evaluate(() => (window as any).PM.hist.undo());
  await expect(cards).toHaveCount(4);
  expect(await page.evaluate(() => (window as any).PM.proj.layers.length)).toBe(4);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('a selection box scrolls through long media lists and Escape clears selection', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await importMedia(page, 12);
  const list = page.getByRole('listbox', { name: 'Project media' });
  const bounds = (await list.boundingBox())!;
  const first = (await page.locator('.asset-card[data-asset-id]').first().boundingBox())!;
  await page.mouse.move(bounds.x + 2, first.y - 2);
  await page.mouse.down();
  await page.mouse.move(bounds.x + bounds.width - 2, bounds.y + bounds.height - 3, { steps: 10 });
  await expect(page.locator('.asset-card[data-asset-id][aria-selected="true"]')).toHaveCount(12);
  expect(await list.evaluate(element => element.scrollTop)).toBeGreaterThan(0);
  await page.mouse.up();
  await page.keyboard.press('Escape');
  await expect(page.locator('.asset-card[data-asset-id][aria-selected="true"]')).toHaveCount(0);
  await expect(page.locator('.asset-card[data-asset-id]')).toHaveCount(12);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
