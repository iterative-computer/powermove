import { expect, test } from './helpers/app';

test.describe('@prompt-glow run halo around the prompt being answered', () => {
  test('wraps the live prompt, stays out of the way, and settles out when the run lands', async ({ session }) => {
    const { page } = session;
    await page.evaluate(() => {
      const PM = (window as any).PM;
      if (!PM.Layout.findPanel(PM.WS.current, 'agent')) {
        const dock = PM.WS.current.layout.docks.find((d: any) => d.id !== 'center') ?? PM.WS.current.layout.docks[0];
        PM.WS.mutate((w: any) => PM.Layout.addPanel(w, 'agent', dock.id));
      }
      // Deterministic stalled run; no real Codex call or user-data writes.
      PM.CodexBridge.request = (prompt: string, _schema: any, _images: any, options: any) => new Promise((resolve, reject) => {
        (window as any).__glowRun = { prompt, options, resolve, reject };
      });
      PM.CodexBridge.steer = async () => true;
    });
    const panel = page.locator('#panel-agent [data-svelte-panel="agent"]');
    await expect(panel).toHaveCount(1);
    const glow = panel.locator('.agent-prompt-glow');
    await expect(glow).toHaveCount(0);

    await panel.locator('textarea').fill('Make the timeline controls clearer');
    await panel.locator('button.agent-send').click();
    await page.waitForFunction(() => Boolean((window as any).__glowRun));

    await expect(glow).toHaveCount(1);
    // No stand-in prose while the trace is empty: the halo is the whole signal.
    await expect(panel.locator('.agent-trace')).toHaveCount(0);
    // The Motion GPU halo reports itself only once it has presented a frame, so
    // this is also the check that the WGSL compiles on a real device.
    await expect(glow).toHaveAttribute('data-glow-renderer', 'motion-gpu', { timeout: 10_000 });

    const geometry = await glow.evaluate(element => {
      const halo = element.getBoundingClientRect();
      const bubble = element.parentElement!.querySelector('.agent-bubble')!.getBoundingClientRect();
      const canvas = element.querySelector('canvas')!.getBoundingClientRect();
      return {
        // Held clear of the bubble on every side.
        left: Math.round(bubble.left - halo.left),
        right: Math.round(halo.right - bubble.right),
        top: Math.round(bubble.top - halo.top),
        bottom: Math.round(halo.bottom - bubble.bottom),
        canvasFills: Math.round(canvas.width - halo.width) + Math.round(canvas.height - halo.height),
        // Atmosphere never takes a click or a selection from the prompt.
        clickThrough: document.elementFromPoint(halo.left + 4, halo.top + 4) !== element
      };
    });
    expect(geometry).toEqual({ left: 26, right: 26, top: 26, bottom: 26, canvasFills: 0, clickThrough: true });

    // A steering follow-up hands the halo to the prompt now being answered.
    // Mid-run the send control is the stop button, so steering goes in on Enter.
    await panel.locator('textarea').fill('Actually make them larger too');
    await panel.locator('textarea').press('Enter');
    await expect(glow).toHaveCount(1);
    await expect.poll(async () => glow.evaluate(element =>
      element.parentElement!.querySelector('.agent-bubble')!.textContent
    )).toBe('Actually make them larger too');

    await page.evaluate(() => {
      (window as any).__glowRun.resolve({
        text: JSON.stringify({ summary: 'Done', commands: [], artifacts: [], externalActions: [], notes: [] })
      });
    });
    // The halo fades out rather than blinking off, and takes its canvas with it
    // when it goes, so a finished run costs no frames.
    await expect(glow).toHaveCount(1);
    await expect(glow).toHaveCount(0, { timeout: 5_000 });
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
});
