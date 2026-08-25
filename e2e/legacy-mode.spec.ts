import { expect, test } from './helpers/app';

test.describe('@legacy-mode the escape hatch still boots the legacy chrome', () => {
  test('shellLegacy store key disables every Svelte engine', async ({ session }) => {
    await session.page.evaluate(async () => {
      (window as any).PM.store.set('shellLegacy', true);
      await (window as any).powermove.store.flush();
    });
    await session.relaunch();
    const { page } = session;
    await page.waitForFunction(() => Boolean((window as any).PM?.GL?.gl));
    const state = await page.evaluate(() => ({
      svelteShell: (window as any).PM.SvelteShell === true,
      titlebarSvelte: document.querySelector('#titlebar')?.getAttribute('data-svelte-shell'),
      panels: document.querySelectorAll('#body .panel').length
    }));
    expect(state.svelteShell).toBe(false);
    expect(state.titlebarSvelte).toBeNull();
    expect(state.panels).toBeGreaterThan(0);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
});
