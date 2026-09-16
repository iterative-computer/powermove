import { test, expect } from './helpers/app';

test('slash shortcuts support keyboard selection, dismissal, pointer selection, and multiline drafts', async ({ session }, testInfo) => {
  await session.openEditor();
  const { page } = session;
  const input = page.getByRole('textbox', { name: 'Message Powermove agent', exact: true });
  await page.evaluate(() => {
    const PM = (window as any).PM;
    // Exercise the persistent composer without depending on account status
    // in the test's fresh profile.
    PM.AgentUI.state.conversation = [{ role: 'assistant', text: 'Ready for your next change.' }];
    (window as any).__composerActions = [];
    PM.AgentUI.setModel = (model: string, effort: string) => (window as any).__composerActions.push({ model, effort });
    PM.AgentUI.setProvider = (provider: string) => (window as any).__composerActions.push({ provider });
  });
  await input.waitFor();
  await input.fill('/');
  const menu = page.getByRole('listbox', { name: 'Slash commands' });
  await expect(menu).toBeVisible();
  await expect(input).toBeFocused();
  await page.screenshot({ path: testInfo.outputPath('slash-commands.png') });
  await input.press('ArrowUp');
  await expect(menu.getByRole('option').last()).toHaveAttribute('aria-selected', 'true');
  await input.press('Escape');
  await expect(menu).toHaveCount(0);
  await expect(input).toHaveValue('/');

  await input.fill('/model');
  await input.press('Tab');
  await expect(input).toHaveValue('/model ');
  await expect(menu.getByRole('option').first()).toBeVisible();
  await input.press('Enter');
  await expect(input).toHaveValue('');
  await expect(input).toBeFocused();
  expect(await page.evaluate(() => (window as any).__composerActions.length)).toBe(1);

  await input.fill('/provider ');
  await menu.getByRole('option').last().click();
  await expect(input).toHaveValue('');
  await expect(input).toBeFocused();
  expect(await page.evaluate(() => (window as any).__composerActions.length)).toBe(2);

  await input.fill('/path/to/file.wav');
  await expect(menu).toHaveCount(0);
  await input.fill('First line');
  await input.press('Shift+Enter');
  await input.pressSequentially('Second line');
  await expect(input).toHaveValue('First line\nSecond line');
  await input.fill('');
  await expect(page.locator('.agent-send')).toBeDisabled();
  expect(session.diagnostics.pageErrors).toEqual([]);
});
