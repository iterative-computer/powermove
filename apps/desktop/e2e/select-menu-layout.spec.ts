import { expect, test } from './helpers/app';

test('a single-option menu fits without a scrollbar near the bottom edge', async ({ session }, testInfo) => {
  await session.openEditor();
  const { page } = session;
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.evaluate(() => {
    const select = document.createElement('select');
    select.setAttribute('aria-label', 'Single model');
    select.innerHTML = '<option>gpt-6-astra</option>';
    Object.assign(select.style, { position: 'fixed', bottom: '44px', left: '200px' });
    document.body.append(select);
  });
  await page.getByRole('combobox', { name: 'Single model', exact: true }).click();
  const menu = page.locator('.pm-menu[role="listbox"]');
  await expect(menu).toBeVisible();
  await menu.screenshot({ path: testInfo.outputPath('single-model.png') });
  const dimensions = await menu.evaluate(el => ({ scroll: el.scrollHeight, height: el.clientHeight }));
  expect(dimensions.scroll).toBeLessThanOrEqual(dimensions.height);
});
