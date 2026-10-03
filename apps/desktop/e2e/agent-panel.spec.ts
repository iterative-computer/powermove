import { expect, test } from './helpers/app';

test.describe('@agent-panel Svelte agent panel drives a real editor-mode run', () => {
  test('opens under the titlebar launcher, never in a dock, and follows the window', async ({ session }) => {
    await session.openEditor(); await session.openAgent();
    const { page } = session;
    const panel = page.locator('#agent-popover #panel-agent');
    await expect(panel).toBeVisible();
    const geometry = () => page.evaluate(() => {
      const launcher = document.getElementById('agent-launcher')!.getBoundingClientRect();
      const popover = document.getElementById('agent-popover')!.getBoundingClientRect();
      return { gap: Math.round(popover.top - launcher.bottom), centred: Math.abs((popover.left + popover.width / 2) - (launcher.left + launcher.width / 2)) < 1 || popover.right >= innerWidth - 13,
        inside: popover.left >= 11 && popover.right <= innerWidth - 11 && popover.bottom <= innerHeight - 11, width: Math.round(popover.width) };
    });
    expect(await geometry()).toMatchObject({ gap: 6, centred: true, inside: true, width: 440 });
    expect(await page.evaluate(() => {
      const PM = (window as any).PM;
      return { docked: !!PM.Layout.findPanel(PM.WS.current, 'agent'), inDock: !!document.querySelector('.dock #panel-agent') };
    })).toEqual({ docked: false, inDock: false });
    await expect(page.locator('#agent-launcher')).toHaveAttribute('aria-expanded', 'true');
    await page.keyboard.press('Escape');
    await expect(panel).toBeHidden();
    await expect(page.locator('#agent-launcher')).toHaveAttribute('aria-expanded', 'false');
    await page.locator('#agent-launcher').click();
    await expect(panel).toBeVisible();
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('dragging the launcher docks the agent; closing the panel returns it to the popover', async ({ session }) => {
    await session.openEditor();
    const { page } = session;
    const launcher = page.locator('#agent-launcher');
    const from = (await launcher.boundingBox())!;
    const to = (await page.locator('#body > .dock#dock-right').boundingBox())!;
    await page.mouse.move(from.x + from.width / 2, from.y + from.height / 2);
    await page.mouse.down();
    await page.mouse.move(from.x + 40, from.y + 80, { steps: 6 });
    await page.mouse.move(to.x + to.width / 2, to.y + to.height - 40, { steps: 12 });
    await page.mouse.up();
    const docked = page.locator('#body > .dock #panel-agent');
    await expect(docked).toBeVisible();
    expect(await page.evaluate(() => (window as any).PM.Layout.findPanel((window as any).PM.WS.current, 'agent')?.dock.id)).toBe('right');
    await expect(page.locator('#agent-popover')).toBeHidden();
    // Pinned to the workspace, the panel replaces the launcher; ⌘⇧K focuses it.
    await expect(launcher).toHaveCount(0);
    await page.evaluate(() => (window as any).PM.SpatialAssistant.open());
    await expect(page.locator('#agent-popover')).toBeHidden();
    await expect(docked.locator('.agent-inline-prompt')).toBeFocused();
    // The projects home has no docks, so the launcher returns there.
    await page.evaluate(() => (window as any).PM.ProjectsScreen.show());
    await expect(launcher).toBeVisible();
    await page.evaluate(() => (window as any).PM.ProjectsScreen.hide());
    await expect(launcher).toHaveCount(0);
    await page.evaluate(() => { const PM = (window as any).PM; PM.WS.mutate((ws: any) => PM.Layout.closePanel(ws, 'agent')); });
    await expect(docked).toHaveCount(0);
    await expect(launcher).toBeVisible();
    await session.openAgent();
    await expect(page.locator('#agent-popover #panel-agent')).toBeVisible();
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('composer submit → progress → result phase, all through the Svelte UI', async ({ session }) => {
    test.skip(!process.env.POWERMOVE_E2E_LIVE, 'live codex run — set POWERMOVE_E2E_LIVE=1 (excluded from the 2-minute CI suite)');
    test.setTimeout(240_000);
    await session.openEditor(); await session.openAgent();
    const { page } = session;
    const panel = page.locator('#agent-popover #panel-agent [data-svelte-panel="agent"]');
    await expect(panel).toHaveCount(1);

    const composer = panel.getByRole('textbox', { name: 'Message Powermove agent', exact: true });
    await expect(composer).toHaveCount(1);
    await composer.fill('Set the selected layer opacity to 50. Reply with a single set_property command.');
    await panel.locator('button.agent-send').click(); await session.openAgent();

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
