import { expect, test } from './helpers/app';

test('New project background follows dragging and restores on Escape', async ({ session }) => {
  const { page } = session;
  await page.evaluate(() => (window as any).PM.newProject());
  const dialog = page.getByRole('dialog', { name: 'New Project', exact: true });
  const background = dialog.locator('.color-field');
  const original = await background.textContent();
  await background.click();
  const picker = page.getByRole('dialog', { name: 'Background', exact: true });
  await picker.evaluate(element => Promise.all(element.getAnimations().map(animation => animation.finished)));
  const plane = picker.getByRole('slider', { name: 'Saturation and brightness' });
  const bounds = (await plane.boundingBox())!;
  await page.mouse.move(bounds.x + bounds.width * 0.25, bounds.y + bounds.height * 0.25);
  await page.mouse.down();
  await expect(background).not.toHaveText(original!);
  const first = await background.textContent();
  await page.mouse.move(bounds.x + bounds.width * 0.75, bounds.y + bounds.height * 0.5, { steps: 5 });
  await expect(background).not.toHaveText(first!);
  await page.mouse.up();
  await page.keyboard.press('Escape');
  await expect(picker).toHaveCount(0);
  await expect(background).toHaveText(original!);

  await background.click();
  await picker.getByRole('textbox', { name: 'Background hex value' }).fill('#AABBCC');
  await picker.getByRole('button', { name: 'Close', exact: true }).click();
  await expect(picker).toHaveCount(0);
  await expect(background).toContainText('#AABBCC');
  await background.click();
  await expect(picker.getByRole('textbox', { name: 'Background hex value' })).toHaveValue('#AABBCC / 100%');
  await picker.getByRole('button', { name: 'Close', exact: true }).click();
  await dialog.getByRole('button', { name: 'Create', exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).PM.proj.bg)).toBe('#AABBCC');
  expect(session.diagnostics.pageErrors).toEqual([]);
});
