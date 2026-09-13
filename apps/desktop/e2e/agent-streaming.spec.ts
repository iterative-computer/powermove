import { test, expect } from './helpers/app';

test('streamed text stays inline and keeps existing nodes as chunks arrive', async ({ session }) => {
  await session.openEditor();
  const page = session.page;
  await page.evaluate(() => {
    const PM = (window as any).PM;
    (window as any).__agentAnimations = [];
    const animate = Element.prototype.animate;
    Element.prototype.animate = function (frames, options) {
      (window as any).__agentAnimations.push({ text: this.textContent, prompt: this.classList.contains('agent-prompt'), frames, options });
      return animate.call(this, frames, options);
    };
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: PM.mkProject({ name: 'Agent streaming' }) }));
    PM.ProjectsScreen.hide();
    PM.SpatialAssistant.open();
    PM.AgentHarness.observe = async () => ({ state: {}, times: [], images: [] });
    PM.CodexBridge.request = async (_p: unknown, _s: unknown, _i: unknown, options: any) => {
      (window as any).__streamChunk = (text: string) => options.onTrace({ kind: 'answer', text });
      return new Promise(() => {});
    };
  });
  await page.getByRole('textbox', { name: 'Message Powermove agent', exact: true }).fill('Stream a reply');
  await page.getByRole('button', { name: 'Send message', exact: true }).click();
  await expect.poll(() => page.evaluate(() => typeof (window as any).__streamChunk)).toBe('function');
  expect(await page.evaluate(() => (window as any).__agentAnimations.some((a: any) => a.prompt && a.frames[0].transform.includes('translateY')))).toBe(true);
  await page.evaluate(() => (window as any).__streamChunk('Hello '));
  const text = page.locator('.agent-trace-text');
  await expect(text).toHaveText('Hello ');
  await text.evaluate((p) => { (window as any).__firstWord = p.firstChild; });
  await page.evaluate(() => (window as any).__streamChunk('world with care'));
  await expect(text).toHaveText('Hello world with care');
  expect(await text.evaluate((p) => p.firstChild === (window as any).__firstWord)).toBe(true);
  expect(await text.locator('span').evaluateAll((nodes) => nodes.every((n) => {
    const css = getComputedStyle(n);
    return css.display === 'inline' && css.transform === 'none' && css.filter === 'none';
  }))).toBe(true);
  const reveals = await page.evaluate(() => (window as any).__agentAnimations.filter((a: any) => ['world', 'with', 'care'].includes(a.text)));
  expect(reveals.map((a: any) => a.options.delay)).toEqual([0, 28, 56]);
  expect(await page.evaluate(() => (window as any).__agentAnimations.filter((a: any) => a.text === 'Hello').length)).toBe(1);
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.evaluate(() => (window as any).__streamChunk(' again'));
  await expect(text).toHaveText('Hello world with care again');
  await expect.poll(() => text.evaluate((p) => p.getAnimations({ subtree: true }).length)).toBe(0);
});
