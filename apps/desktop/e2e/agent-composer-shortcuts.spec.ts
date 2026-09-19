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
      await expect(input).toHaveText('Keep this draft ');
      await destination.click({ position: { x: 40, y: 40 } });
      await page.keyboard.press('Space');
      await expect(input).toHaveText('Keep this draft ');
      await expect(input).not.toBeFocused();
      expect(await page.evaluate(() => (window as any).__playbackToggles)).toBe(attempt + 1);
    }
    await input.fill('/');
    await input.press('Escape');
    await expect(input).toBeFocused();
    await input.press('Escape');
    await expect(input).not.toBeFocused();
    await page.keyboard.press('Space');
    await expect(input).toHaveText('/');
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
  await expect(input).toHaveText('/');

  await input.fill('/model');
  await input.press('Tab');
  await expect(input).toHaveText('/model ');
  await expect(menu.getByRole('option').first()).toBeVisible();
  await input.press('Enter');
  await expect(input).toHaveText('');
  await expect(input).toBeFocused();
  expect(await page.evaluate(() => (window as any).__composerActions.length)).toBe(1);

  await input.fill('/provider ');
  await menu.getByRole('option').last().click();
  await expect(input).toHaveText('');
  await expect(input).toBeFocused();
  expect(await page.evaluate(() => (window as any).__composerActions.length)).toBe(2);

  await input.fill('/path/to/file.wav');
  await expect(menu).toHaveCount(0);
  await input.fill('First line');
  await input.press('Shift+Enter');
  await input.pressSequentially('Second line');
  await expect(input).toHaveText('First line\nSecond line');
  await input.fill('');
  await expect(page.locator('.agent-send')).toBeDisabled();
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('inline file tokens preserve their place, bytes, and two-step deletion in the desktop editor', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.AgentUI.state.conversation = [{ role: 'assistant', text: 'Ready.' }];
  });
  const input = page.getByRole('textbox', { name: 'Message Powermove agent', exact: true });
  await input.fill('Use this ');
  await input.press('End');
  await page.evaluate(async () => { await (window as any).PM.AgentUI.addAttachments([new File(['fixture bytes'], 'backgrounds.aep')]); });
  const token = input.locator('[data-attachment-id]');
  await expect(token).toHaveCount(1);
  await expect(token.locator('.agent-inline-attachment-type')).toHaveText('AEP');
  await input.pressSequentially(' as a reference');
  const draft = await page.evaluate(() => {
    const state = (window as any).PM.AgentUI.state;
    return { text: state.composerDraft, offset: state.attachments[0].promptOffset, bytes: state.attachments[0].dataBase64 };
  });
  expect(draft).toEqual({ text: 'Use this  as a reference', offset: 9, bytes: 'Zml4dHVyZSBieXRlcw==' });
  // Select-all then Right collapses to the end; remove only the suffix text.
  await input.press('ControlOrMeta+ArrowRight');
  for (let i = 0; i < ' as a reference'.length; i++) await input.press('Backspace');
  await input.press('Backspace');
  await expect(token).toHaveCount(1);
  await expect(token).toHaveClass(/is-selected/);
  await input.press('Backspace');
  await expect(token).toHaveCount(0);
  await input.press('ControlOrMeta+z');
  await expect(token).toHaveCount(1);
  expect(await page.evaluate(() => (window as any).PM.AgentUI.state.attachments[0].dataBase64)).toBe(draft.bytes);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('file-only drafts keep their caret and new text on the same line', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await page.evaluate(() => { (window as any).PM.AgentUI.state.conversation = [{ role: 'assistant', text: 'Ready.' }]; });
  const input = page.getByRole('textbox', { name: 'Message Powermove agent', exact: true });
  await input.fill('Before ');
  await input.press('End');
  await page.evaluate(async () => { await (window as any).PM.AgentUI.addAttachments([new File(['fixture'], 'backgrounds.aep')]); });
  const token = input.locator('[data-attachment-id]');
  await expect(token).toHaveCount(1);
  // Delete the full prefix with the native editing path that previously added <br>.
  await input.evaluate(element => {
    const range = document.createRange(); range.selectNodeContents(element.firstChild!);
    const selection = window.getSelection()!; selection.removeAllRanges(); selection.addRange(range);
  });
  await page.keyboard.press('Backspace');
  await expect(input.locator('br')).toHaveCount(0);
  await page.keyboard.type('Before ');
  const bounds = await token.boundingBox();
  await page.mouse.click(bounds!.x + bounds!.width + 5, bounds!.y + bounds!.height / 2);
  await page.keyboard.type(' after');
  await expect.poll(() => page.evaluate(() => (window as any).PM.AgentUI.state.composerDraft)).toBe('Before  after');
  await expect(input.locator('br')).toHaveCount(0);
  const lineTops = await input.evaluate(element => [...element.childNodes].filter(node => node.textContent?.trim()).map(node => {
    const range = document.createRange(); range.selectNodeContents(node); return range.getBoundingClientRect().top;
  }));
  expect(Math.max(...lineTops) - Math.min(...lineTops)).toBeLessThan(5);
  await page.keyboard.press('Shift+Enter');
  await page.keyboard.type('Next line');
  await expect.poll(() => page.evaluate(() => (window as any).PM.AgentUI.state.composerDraft)).toBe('Before  after\nNext line');
  expect(session.diagnostics.pageErrors).toEqual([]);
});
