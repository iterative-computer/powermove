import { test, expect } from './helpers/app';

test('agent content wraps without horizontal scroll at narrow and normal widths', async ({ session }) => {
  await session.openEditor();
  const page = session.page;
  const token = 'long_command_or_path_'.repeat(60);
  await page.evaluate((token) => {
    const PM = (window as any).PM;
    PM.SpatialAssistant.open();
    PM.AgentHarness.observe = async () => ({ state: {}, times: [], images: [] });
    PM.CodexBridge.request = async (_p: unknown, _s: unknown, _i: unknown, options: any) => {
      options.onTrace({ kind: 'answer', text: `Output ${token}\n\`${token}\`` });
      options.onTrace({ kind: 'tool-start', id: 'width-tool', toolName: 'bash', label: token });
      return new Promise(() => {});
    };
  }, token);
  const composer = page.getByRole('textbox', { name: 'Message Powermove agent', exact: true });
  await composer.fill(token);
  await page.getByRole('button', { name: 'Send message', exact: true }).click();
  await expect(page.locator('.agent-tool-details')).toBeVisible();
  await composer.fill(token);
  for (const width of [240, 320, 480]) {
    await page.locator('#panel-agent').evaluate((panel, width) => {
      Object.assign((panel as HTMLElement).style, { width: `${width}px`, maxWidth: `${width}px`, minWidth: '0' });
    }, width);
    const overflow = await page.locator('.agent-shell').evaluate((root) =>
      [root, ...root.querySelectorAll('*')].filter((el) => {
        const css = getComputedStyle(el);
        return el.clientWidth > 0 && !el.classList.contains('panel-sr-only') && css.display !== 'inline' && css.textOverflow !== 'ellipsis'
          // Keep checking the text and scroll container; the halo extends beyond its wrappers.
          && !(el.matches('.agent-prompt, .agent-msg.user') && el.querySelector('.agent-prompt-signal'))
          && el.scrollWidth > el.clientWidth + 1;
      }).map((el) => ({ tag: el.tagName, class: el.className, width: el.clientWidth, scroll: el.scrollWidth })));
    expect(overflow, `overflow at ${width}px`).toEqual([]);
    const scroll = page.locator('.agent-scroll');
    expect(await scroll.evaluate((el) => {
      el.scrollLeft = 10000;
      el.scrollTop = 10000;
      return { x: el.scrollLeft, vertical: el.scrollHeight > el.clientHeight, y: el.scrollTop > 0 };
    })).toEqual({ x: 0, vertical: true, y: true });
  }
  expect(await page.locator('.agent-tool-details span').textContent()).toBe(token);
});
