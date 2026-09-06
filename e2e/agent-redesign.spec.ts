import { mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { test, expect } from './helpers/app';

test('studio agent keeps suggestions, steering, activity and narrow layouts usable', async ({ session }, testInfo) => {
  const { page } = session;
  const artifacts = process.env.POWERMOVE_AGENT_ARTIFACTS || testInfo.outputPath('visuals');
  await mkdir(artifacts, { recursive: true });
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.theme.apply('dark');
    PM.SpatialAssistant.open();
    PM.AgentUI.setAccess('editor');
    PM.AgentUI.setModel('gpt-6-astra', 'high');
  });
  const panel = page.locator('#panel-agent');
  const composer = page.getByRole('textbox', { name: 'Message Powermove agent', exact: true });
  await expect(page.getByText('Make your next move', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Animate a title', exact: true }).click();
  await expect(composer).toHaveValue(/editable keyframes/);
  await expect(composer).toBeFocused();
  await composer.fill('');
  await page.screenshot({ path: path.join(artifacts, 'agent-empty-dark.png') });

  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.AgentUI.setAccess('project');
    PM.AgentHarness.observe = async () => ({ state: {}, times: [], images: [] });
    PM.CodexBridge.request = async (_p: unknown, _s: unknown, _i: unknown, options: any) => {
      (window as any).__redesignTrace = options.onTrace;
      options.onTrace({ kind: 'answer', text: 'I’ll animate the title and keep every property editable.' });
      options.onTrace({ kind: 'tool-start', itemId: 'inspect', toolName: 'read_file', label: 'Read title properties and existing keyframes' });
      options.onTrace({ kind: 'tool-end', itemId: 'inspect', isError: false });
      options.onTrace({ kind: 'tool-start', itemId: 'preview', toolName: 'bash', label: 'Check the first preview render' });
      options.onTrace({ kind: 'tool-end', itemId: 'preview', isError: true, detail: 'Preview unavailable' });
      options.onTrace({ kind: 'tool-start', itemId: 'animate', toolName: 'edit_file', label: 'Add entrance keyframes to title position and opacity' });
      return new Promise(resolve => { (window as any).__redesignFinish = resolve; });
    };
    (window as any).__redesignSteers = [];
    PM.CodexBridge.steer = async (prompt: string) => { (window as any).__redesignSteers.push(prompt); return true; };
  });
  await composer.fill('Give the title a confident entrance');
  await page.getByRole('button', { name: 'Send message', exact: true }).click();
  await expect(page.locator('.agent-tool-details')).toBeVisible();
  await expect(page.locator('.agent-tool-details .is-failed')).toContainText('Check the first preview render');
  await expect(page.getByRole('button', { name: 'Stop current run', exact: true })).toBeVisible();
  await page.screenshot({ path: path.join(artifacts, 'agent-working-dark.png') });
  await expect.poll(() => page.locator('.atmosphere-field').getAttribute('data-atmosphere-renderer')).toMatch(/^(motion-gpu|static)$/);
  await writeFile(path.join(artifacts, 'renderer-evidence.json'), JSON.stringify(await page.evaluate(() => ({
    hidden: document.hidden,
    gpu: 'gpu' in navigator,
    renderer: (document.querySelector('.atmosphere-field') as HTMLElement)?.dataset,
    canvas: [...document.querySelectorAll('.atmosphere-field canvas')].map(c => ({ width: (c as HTMLCanvasElement).width, height: (c as HTMLCanvasElement).height })),
  })), null, 2));
  await panel.evaluate(el => (el as HTMLElement).style.setProperty('--set-panel-height', '580px'));
  await page.locator('.agent-scroll').evaluate(el => { el.scrollTop = 0; });
  await expect.poll(() => page.locator('.agent-prompt-signal').getAttribute('data-glow-renderer')).toBe('motion-gpu');
  await expect(page.locator('.agent-prompt-signal canvas')).toHaveCount(1);
  // Capture the settled 450 ms shader entrance, rather than its transparent first frame.
  await page.waitForTimeout(600);
  await panel.screenshot({ path: path.join(artifacts, 'agent-working-expanded.png') });
  await panel.evaluate(el => (el as HTMLElement).style.removeProperty('--set-panel-height'));

  await page.emulateMedia({ reducedMotion: 'reduce' });
  await expect(page.locator('.atmosphere-field canvas')).toHaveCount(0);
  await expect(page.locator('.agent-prompt-signal canvas')).toHaveCount(0);
  await expect(page.locator('.agent-prompt-signal')).toHaveAttribute('data-glow-fallback', 'reduced-motion');
  await expect.poll(() => page.locator('.agent-tool-activity').evaluate(el => el.getAnimations({ subtree: true }).length)).toBe(0);
  await page.emulateMedia({ reducedMotion: 'no-preference' });
  for (const width of [240, 320, 480]) {
    await panel.evaluate((el, width) => {
      Object.assign((el as HTMLElement).style, { width: `${width}px`, maxWidth: `${width}px`, minWidth: '0' });
      (el.closest('.dock') as HTMLElement).style.flex = `0 0 ${width}px`;
    }, width);
    await composer.fill('Keep the entrance subtle');
    await expect(page.getByRole('button', { name: 'Steer current run', exact: true })).toBeVisible();
    const overflow = await page.locator('.agent-shell').evaluate(root => [root, ...root.querySelectorAll('*')]
      .filter(el => el.clientWidth > 0 && !el.classList.contains('panel-sr-only')
        && getComputedStyle(el).display !== 'inline' && getComputedStyle(el).textOverflow !== 'ellipsis'
        && el.scrollWidth > el.clientWidth + 1)
      .map(el => el.className));
    expect(overflow, `horizontal overflow at ${width}px`).toEqual([]);
    await panel.screenshot({ path: path.join(artifacts, `agent-working-${width}.png`) });
  }
  await page.getByRole('button', { name: 'Steer current run', exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).__redesignSteers.length)).toBe(1);
  await expect(page.locator('.agent-trace.is-archived .agent-trace-text')).toContainText('every property editable');

  await page.evaluate(() => {
    (window as any).__redesignTrace({ kind: 'tool-end', itemId: 'animate', isError: false });
    (window as any).__redesignFinish({ text: JSON.stringify({
      summary: 'The title entrance is ready. Position and opacity remain editable.',
      commands: [], artifacts: [], externalActions: [], notes: []
    }) });
  });
  await expect(page.locator('.agent-msg.assistant p')).toContainText('The title entrance is ready.');
  await expect(page.locator('.agent-prompt-signal')).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Send message', exact: true })).toBeVisible();
  await panel.evaluate(el => {
    for (const name of ['width', 'max-width', 'min-width']) (el as HTMLElement).style.removeProperty(name);
    (el.closest('.dock') as HTMLElement).style.flex = '0 0 320px';
  });
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.evaluate(() => { (window as any).PM.theme.apply('light'); });
  const jumpToLatest = page.getByRole('button', { name: 'Scroll to latest', exact: true });
  if (await jumpToLatest.isVisible()) await jumpToLatest.click();
  await expect(page.locator('.agent-msg.assistant p')).toBeInViewport();
  await panel.screenshot({ path: path.join(artifacts, 'agent-complete-light.png') });
  expect(session.diagnostics.pageErrors).toEqual([]);
});
