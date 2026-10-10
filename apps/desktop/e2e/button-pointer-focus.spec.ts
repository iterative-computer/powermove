import { expect, test } from './helpers/app';

test('clicking a button does not leave focus or assign Space to its action', async ({ session }) => {
  const { page } = session;
  await session.openEditor();
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.__buttonClicks = 0;
    PM.__playPresses = 0;
    PM.newProject = () => { PM.__buttonClicks++; };
    PM.toggle = () => { PM.__playPresses++; };
  });
  const button = page.getByRole('button', { name: 'New project', exact: true });
  await button.click();
  await expect(button).not.toBeFocused();
  await page.keyboard.press('Space');
  expect(await page.evaluate(() => ({ clicks: (window as any).PM.__buttonClicks, plays: (window as any).PM.__playPresses })))
    .toEqual({ clicks: 1, plays: 1 });

  // Keyboard navigation still focuses controls and Enter still activates them.
  await button.focus();
  await expect(button).toBeFocused();
  await page.keyboard.press('Enter');
  expect(await page.evaluate(() => (window as any).PM.__buttonClicks)).toBe(2);

  // Clicking a button also leaves the text-entry shortcut context.
  await page.evaluate(() => {
    const input = document.createElement('input');
    input.id = 'pointer-focus-field';
    document.body.append(input);
    input.focus();
  });
  await button.click();
  await expect(page.locator('#pointer-focus-field')).not.toBeFocused();
  await page.keyboard.press('Space');
  expect(await page.evaluate(() => (window as any).PM.__playPresses)).toBe(2);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
