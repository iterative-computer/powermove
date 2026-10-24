import { expect, test } from './helpers/app';

test('status contributions keep their foreground cadence and stop polling behind Home and Settings', async ({ session }) => {
  const { page } = session;
  await page.evaluate(async () => {
    const PM = (window as any).PM;
    await PM.Kernel.loader.builtinsReady;
    const state = { ticks: 0, textCalls: 0, label: 'Status clock initial', original: PM.invalidate, handle: null as any };
    (window as any).__statusPolling = state;
    PM.invalidate = function (...args: any[]) {
      if (args[0] === 'status') state.ticks++;
      return state.original.apply(this, args);
    };
    state.handle = PM.Kernel.status.register('status-polling-test', {
      id: 'status-polling-clock', text: () => { state.textCalls++; return state.label; },
    });
  });
  try {
    await expect(page.locator('#status')).toContainText('Status clock initial');
    expect(await page.evaluate(() => (window as any).PM.ProjectsScreen.isOpen)).toBe(true);
    const home = await page.evaluate(() => {
      const state = (window as any).__statusPolling; state.ticks = 0;
      return state.textCalls;
    });
    await page.waitForTimeout(2200);
    expect(await page.evaluate(() => (window as any).__statusPolling.ticks)).toBe(0);
    expect(await page.evaluate(() => (window as any).__statusPolling.textCalls)).toBe(home);

    await session.openEditor();
    // The hidden native harness disables background throttling so its document
    // remains visible. Verify that prerequisite before checking foreground work.
    expect(await page.evaluate(() => document.hidden)).toBe(false);
    await page.evaluate(() => { (window as any).__statusPolling.ticks = 0; });
    await expect.poll(() => page.evaluate(() => (window as any).__statusPolling.ticks)).toBeGreaterThanOrEqual(1);

    await page.evaluate(() => (window as any).PM.SettingsUI.open('general'));
    await expect(page.locator('#settings-screen')).toBeVisible();
    const settings = await page.evaluate(() => {
      const state = (window as any).__statusPolling; state.ticks = 0;
      state.label = 'Status clock returned'; return state.textCalls;
    });
    await page.waitForTimeout(2200);
    expect(await page.evaluate(() => (window as any).__statusPolling.ticks)).toBe(0);
    expect(await page.evaluate(() => (window as any).__statusPolling.textCalls)).toBe(settings);
    await page.evaluate(() => (window as any).PM.SettingsUI.close());
    await expect(page.locator('#status')).toContainText('Status clock returned', { timeout: 500 });
    await page.evaluate(() => { (window as any).__statusPolling.ticks = 0; });
    await expect.poll(() => page.evaluate(() => (window as any).__statusPolling.ticks)).toBeGreaterThanOrEqual(1);
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally {
    await page.evaluate(() => {
      const state = (window as any).__statusPolling;
      if (!state) return;
      (window as any).PM.invalidate = state.original; state.handle?.dispose();
      delete (window as any).__statusPolling;
    });
  }
});
