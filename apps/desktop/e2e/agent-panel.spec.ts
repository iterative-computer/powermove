import { expect, test } from './helpers/app';

test.describe('@agent-panel Svelte agent panel drives a real editor-mode run', () => {
  test('keeps a set height when the editor window grows', async ({ session }) => {
    await session.openEditor();
    const { page } = session;
    await page.setViewportSize({ width: 1280, height: 760 });
    const panel = page.locator('#panel-agent');
    await expect(panel).toBeVisible();
    await expect.poll(async () => Math.round((await panel.boundingBox())?.height ?? 0)).toBe(350);

    await page.setViewportSize({ width: 1440, height: 900 });
    await expect.poll(async () => Math.round((await panel.boundingBox())?.height ?? 0)).toBe(350);
    expect(await page.evaluate(() => {
      const PM = (window as any).PM;
      const agent = PM.Layout.findPanel(PM.WS.current, 'agent')?.spec;
      return { size: agent?.size, flex: agent?.flex };
    })).toEqual({ size: 350, flex: undefined });
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('composer submit → progress → result phase, all through the Svelte UI', async ({ session }) => {
    test.skip(!process.env.POWERMOVE_E2E_LIVE, 'live codex run — set POWERMOVE_E2E_LIVE=1 (excluded from the 2-minute CI suite)');
    test.setTimeout(240_000);
    await session.openEditor();
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
