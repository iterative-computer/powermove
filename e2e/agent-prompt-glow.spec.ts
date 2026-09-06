import { mkdir } from 'node:fs/promises';
import path from 'node:path';
import { test, expect } from './helpers/app';

test('prompt halo sits outside the message bubble and stays within the panel', async ({ session }, testInfo) => {
  const { page } = session;
  const output = process.env.POWERMOVE_AGENT_ARTIFACTS || testInfo.outputPath('visuals');
  await mkdir(output, { recursive: true });
  await page.getByRole('textbox', { name: 'Message Powermove agent', exact: true }).waitFor();
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.theme.apply('dark');
    Object.assign(PM.AgentUI.state, { phase: 'running', legacyPhase: 'working', activity: '', trace: [],
      conversation: [{ role: 'user', text: '"Could not reach Pexels"' }], run: null, panelRun: null });
  });
  const panel = page.locator('#panel-agent');
  for (const width of [240, 320, 480]) {
    await panel.evaluate((el, width) => { (el.closest('.dock') as HTMLElement).style.flex = `0 0 ${width}px`; }, width);
    const bounds = await page.locator('.agent-prompt').evaluate(el => {
      const bubble = el.getBoundingClientRect();
      const halo = el.querySelector('.agent-prompt-signal')!.getBoundingClientRect();
      const panel = el.closest('#panel-agent')!.getBoundingClientRect();
      return { left: bubble.left - halo.left, right: halo.right - bubble.right,
        top: bubble.top - halo.top, bottom: halo.bottom - bubble.bottom,
        contained: halo.left >= panel.left && halo.right <= panel.right };
    });
    expect(bounds.left).toBeGreaterThan(0);
    expect(bounds.right).toBe(bounds.left);
    expect(bounds.top).toBe(bounds.left);
    expect(bounds.bottom).toBe(bounds.left);
    expect(bounds.contained).toBe(true);
    expect(await page.locator('.agent-scroll').evaluate(el => { el.scrollLeft = 1000; return el.scrollLeft; })).toBe(0);
  }
  await expect.poll(() => page.locator('.agent-prompt-signal').getAttribute('data-glow-renderer')).toBe('motion-gpu');
  await page.waitForTimeout(600); // Let the shader entrance settle for visual review.
  await panel.screenshot({ path: path.join(output, 'prompt-halo-dark.png') });
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await expect(page.locator('.agent-prompt-signal')).toHaveAttribute('data-glow-fallback', 'reduced-motion');
  await expect(page.locator('.agent-prompt-signal canvas')).toHaveCount(0);
  await page.evaluate(() => { (window as any).PM.AgentUI.state.phase = 'result'; });
  await expect(page.locator('.agent-prompt-signal')).toHaveCount(0);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
