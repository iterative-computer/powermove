import { test, expect } from './helpers/app';

for (const surface of ['viewer', 'timeline']) {
  test(`Space returns to playback after leaving the agent for the ${surface}`, async ({ session }) => {
    await session.openEditor();
    const { page } = session;
    await page.evaluate(() => {
      const PM = (window as any).PM;
      PM.AgentUI.state.conversation = [{ role: 'assistant', text: 'Ready.' }];
      PM.pause();
      (window as any).__playbackToggles = 0;
      PM.toggle = () => { (window as any).__playbackToggles += 1; };
    });
    const input = page.getByRole('textbox', { name: 'Message Powermove agent', exact: true });
    const destination = page.locator(surface === 'viewer' ? '#viewer-stage, #stage' : '#tl-canvas-wrap canvas').first();
    for (let attempt = 0; attempt < 3; attempt += 1) {
      await input.fill('Keep this draft');
      await input.press('Space');
      await expect(input).toHaveValue('Keep this draft ');
      await destination.click({ position: { x: 40, y: 40 } });
      await page.keyboard.press('Space');
      await expect(input).toHaveValue('Keep this draft ');
      await expect(input).not.toBeFocused();
      expect(await page.evaluate(() => (window as any).__playbackToggles)).toBe(attempt + 1);
    }
    await input.fill('/');
    await input.press('Escape');
    await expect(input).toBeFocused();
    await input.press('Escape');
    await expect(input).not.toBeFocused();
    await page.keyboard.press('Space');
    await expect(input).toHaveValue('/');
    expect(await page.evaluate(() => (window as any).__playbackToggles)).toBe(4);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
}

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
