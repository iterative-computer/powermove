import { expect, test } from './helpers/app';

test.describe('@agent-panel Svelte agent panel drives a real editor-mode run', () => {
  test('composer submit → progress → result phase, all through the Svelte UI', async ({ session }) => {
    test.skip(!process.env.POWERMOVE_E2E_LIVE, 'live codex run — set POWERMOVE_E2E_LIVE=1 (excluded from the 2-minute CI suite)');
    test.setTimeout(240_000);
    const { page } = session;
    await page.evaluate(() => {
      const PM = (window as any).PM;
      if (!PM.Layout.findPanel(PM.WS.current, 'agent')) {
        const dock = PM.WS.current.layout.docks.find((d: any) => d.id !== 'center') ?? PM.WS.current.layout.docks[0];
        PM.WS.mutate((w: any) => PM.Layout.addPanel(w, 'agent', dock.id));
      }
    });
    const panel = page.locator('#panel-agent [data-svelte-panel="agent"]');
    await expect(panel).toHaveCount(1);

    const composer = panel.locator('textarea');
    await expect(composer).toHaveCount(1);
    await composer.fill('Set the selected layer opacity to 50. Reply with a single set_property command.');
    await panel.locator('button.agent-send').click();

    // A real codex editor run: running with streamed progress, then a terminal phase.
    await expect
      .poll(async () => page.evaluate(() => (window as any).PM.AgentUI?.state?.phase ?? null), { timeout: 30_000, intervals: [1000] })
      .toBe('running');
    await expect
      .poll(async () => page.evaluate(() => (window as any).PM.AgentUI?.state?.progressLines?.length ?? 0), { timeout: 60_000, intervals: [2000] })
      .toBeGreaterThan(0);
    await expect
      .poll(async () => page.evaluate(() => (window as any).PM.AgentUI?.state?.phase ?? null), { timeout: 200_000, intervals: [2000] })
      .toMatch(/preview|result|idle|prompt/); // 'prompt' = composer returned to ready (terminal for runs whose plan resolves immediately)
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
});
