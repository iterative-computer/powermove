import { expect, test } from './helpers/app';

for (const accepted of [true, false]) {
  test(`steering keeps reasoning visible with ${accepted ? 'live transport' : 'replacement runs'}`, async ({ session }) => {
    await session.openEditor();
    const { page } = session;
    await page.evaluate(() => {
      const PM = (window as any).PM;
      PM.AgentUI.setAccess('project');
      (window as any).__reasoningRuns = [];
      PM.CodexBridge.request = (_prompt: unknown, _schema: unknown, _images: unknown, options: any) => new Promise(resolve => {
        (window as any).__reasoningRuns.push({ options, resolve });
      });
      PM.CodexBridge.steer = () => new Promise(resolve => { (window as any).__resolveReasoningSteer = resolve; });
    });
    const composer = page.getByRole('textbox', { name: 'Message Powermove agent', exact: true });
    await composer.fill('Inspect the composition');
    await composer.press('Enter');
    await expect.poll(() => page.evaluate(() => (window as any).__reasoningRuns.length)).toBe(1);

    for (let index = 0; index < 2; index++) {
      await page.evaluate(index => {
        (window as any).__resolveReasoningSteer = null;
        (window as any).__reasoningRuns.at(-1).options.onTrace({ kind: 'thought', text: `Inspecting timing ${index}.` });
      }, index);
      await expect(page.locator('.agent-trace.is-live .agent-tool-thought')).toContainText(`Inspecting timing ${index}.`);
      await composer.fill(`Refine timing ${index}`);
      await composer.press('Enter');
      await page.waitForFunction(() => typeof (window as any).__resolveReasoningSteer === 'function');
      const archivedThought = page.locator('.agent-trace.is-archived .agent-trace-text').filter({ hasText: `Inspecting timing ${index}.` });
      // Presence in the DOM is insufficient: a closed work log hides the text.
      await expect(archivedThought).toBeVisible();
      await page.evaluate(({ accepted, index }) => {
        (window as any).__reasoningRuns.at(-1).options.onTrace({ kind: 'thought', text: `Checking alignment ${index}.` });
        (window as any).__resolveReasoningSteer(accepted);
      }, { accepted, index });
      await expect.poll(() => page.evaluate(() => (window as any).__reasoningRuns.length)).toBe(accepted ? 1 : index + 2);
      await expect(archivedThought).toBeVisible();
      if (!accepted) {
        await expect(page.locator('.agent-trace.is-archived .agent-trace-text').filter({ hasText: `Checking alignment ${index}.` })).toBeVisible();
      }
    }

    await page.evaluate(() => {
      (window as any).__reasoningRuns.at(-1).resolve({
        text: JSON.stringify({ summary: 'Timing checked.', commands: [], artifacts: [], externalActions: [], notes: [] }),
        extensions: []
      });
    });
    await expect.poll(() => page.evaluate(() => (window as any).PM.AgentUI.state.legacyPhase)).toBe('result');
    for (let index = 0; index < 2; index++) {
      await expect(page.locator('.agent-trace.is-archived .agent-trace-text').filter({ hasText: `Inspecting timing ${index}.` })).toBeVisible();
    }
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
}
