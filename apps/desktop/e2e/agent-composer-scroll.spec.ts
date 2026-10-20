import { test, expect } from './helpers/app';

for (const surface of ['popover', 'dock'] as const) {
  test(`composer keeps wrapped typing visible in the ${surface}`, async ({ session }) => {
    await session.openEditor();
    const { page } = session;
    await page.setViewportSize({ width: 1440, height: 900 });
    if (surface === 'popover') await session.openAgent();
    else await page.evaluate(() => {
      const PM = (window as any).PM;
      PM.WS.mutate((workspace: any) => PM.Layout.addPanel(workspace, 'agent', 'left'));
    });
    const input = page.getByRole('textbox', { name: 'Message Powermove agent', exact: true });
    const caret = () => input.evaluate(element => {
      const selection = window.getSelection()!;
      const range = selection.getRangeAt(0).cloneRange();
      if (selection.focusNode === element && selection.focusOffset === element.childNodes.length
        && element.lastChild?.nodeType === Node.TEXT_NODE) {
        range.setStart(element.lastChild, element.lastChild.textContent!.length); range.collapse(true);
      }
      // Chromium exposes the empty final line through its placeholder break.
      if (!range.getBoundingClientRect().height && element.querySelector('[data-prompt-tail]')) {
        range.selectNode(element.querySelector('[data-prompt-tail]')!);
      }
      const rect = range.getBoundingClientRect();
      const box = element.getBoundingClientRect();
      return { top: rect.top - box.top, bottom: rect.bottom - box.top, caretHeight: rect.height,
        height: element.clientHeight, scrollTop: element.scrollTop,
        overflow: element.scrollHeight > element.clientHeight };
    });
    const expectCaretVisible = async () => {
      await expect.poll(async () => {
        const position = await caret();
        return { ...position, visible: position.caretHeight > 0 && position.top >= -1 && position.bottom <= position.height + 1 };
      }).toMatchObject({ visible: true });
    };

    for (const theme of ['dark', 'light']) {
      await page.evaluate(theme => (window as any).PM.theme.apply(theme), theme);
      await input.fill('');
      // Ordinary keystrokes, with no arrow key or explicit newline to scroll for us.
      for (let line = 0; line < 10; line++) {
        await input.pressSequentially('Keep the words I am typing visible as this sentence wraps. ');
        await expectCaretVisible();
      }
      expect((await caret()).overflow).toBe(true);
      expect((await caret()).scrollTop).toBeGreaterThan(0);

      // Editing near the beginning must follow that caret, not jump to the end.
      await input.press('ControlOrMeta+a');
      await input.press('ArrowLeft');
      await input.pressSequentially('Start here. ');
      await expectCaretVisible();
      expect((await caret()).scrollTop).toBe(0);

      await input.press('ControlOrMeta+a');
      await input.press('ArrowRight');
      await input.press('Shift+Enter');
      await expectCaretVisible();
      await input.pressSequentially('Last line');
      await expectCaretVisible();
      await input.press('ControlOrMeta+a');
      await input.press('Backspace');
      await expect(input).toHaveText('');
      await input.press('ControlOrMeta+z');
      await expectCaretVisible();
      expect((await caret()).scrollTop).toBeGreaterThan(0);
      await input.press('ControlOrMeta+Shift+z');
      await expect(input).toHaveText('');
      await input.pressSequentially('Short draft');
      await expectCaretVisible();
      expect((await caret()).overflow).toBe(false);
      expect((await caret()).scrollTop).toBe(0);

      await input.evaluate(element => {
        const clipboardData = new DataTransfer();
        clipboardData.setData('text/plain', ' Pasted words that wrap onto another line.'.repeat(20));
        element.dispatchEvent(new ClipboardEvent('paste', { clipboardData, bubbles: true, cancelable: true }));
      });
      await expectCaretVisible();
      expect((await caret()).scrollTop).toBeGreaterThan(0);
    }
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
}
