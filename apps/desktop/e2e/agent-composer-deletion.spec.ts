import { test, expect } from './helpers/app';

test('clearing composer lines preserves only intentional newlines and keeps the caret in place', async ({ session }) => {
  await session.openEditor(); await session.openAgent();
  const { page } = session;
  await page.evaluate(() => { (window as any).PM.AgentUI.state.conversation = [{ role: 'assistant', text: 'Ready.' }]; });
  const input = page.getByRole('textbox', { name: 'Message Powermove agent', exact: true });
  const draft = () => page.evaluate(() => (window as any).PM.AgentUI.state.composerDraft);

  await input.fill('First');
  await input.press('End');
  await input.press('Shift+Enter');
  await input.pressSequentially('Last');
  for (let i = 0; i < 4; i++) await input.press('Backspace');
  await expect.poll(draft).toBe('First\n');
  await input.pressSequentially('Again');
  await expect.poll(draft).toBe('First\nAgain');

  // Clear a whole draft, then type again at the same caret.
  await input.press('ControlOrMeta+a');
  await input.press('Backspace');
  await expect.poll(draft).toBe('');
  await expect(input.locator('br')).toHaveCount(0);
  await input.pressSequentially('Replacement');
  await expect.poll(draft).toBe('Replacement');

  await input.press('Shift+Enter');
  await input.pressSequentially('Last');
  await input.press('Meta+Backspace');
  await expect.poll(draft).toBe('Replacement\n');
  await input.press('ControlOrMeta+z');
  await expect.poll(draft).toBe('Replacement\nLast');
  await input.press('ControlOrMeta+Shift+z');
  await expect.poll(draft).toBe('Replacement\n');
  // Removing the intentional newline still joins the lines normally.
  await input.press('Backspace');
  await expect.poll(draft).toBe('Replacement');

  // Forward-delete the text on an interior line, preserving its two breaks.
  await input.press('Shift+Enter');
  await input.pressSequentially('Middle');
  await input.press('Shift+Enter');
  await input.pressSequentially('End');
  await input.evaluate(element => {
    const text = element.firstChild!;
    const range = document.createRange(); range.setStart(text, 'Replacement\n'.length); range.collapse(true);
    const selection = window.getSelection()!; selection.removeAllRanges(); selection.addRange(range);
  });
  for (let i = 0; i < 6; i++) await input.press('Delete');
  await expect.poll(draft).toBe('Replacement\n\nEnd');
  await input.pressSequentially('New');
  await expect.poll(draft).toBe('Replacement\nNew\nEnd');
  expect(session.diagnostics.pageErrors).toEqual([]);
});
