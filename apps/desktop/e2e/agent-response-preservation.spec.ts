import { expect, test } from './helpers/app';

// The final result summary can be shorter than the public answer. Keep every
// paragraph the user already read, including after sending and loading history.
const paragraphs = [
  'The files are in `/Users/example/Library/Application Support/powermove/Agent Workspaces/example/artifacts/output/`. In Finder, choose Go > Go to Folder and paste that path.',
  'Subfolders: `logos/`, `harnesses/`, `fonts/`, `screens/`, plus `BRAND_NOTES.md`.',
  'I could not open Finder for you.',
  'A second empty folder came from an earlier prompt and can be ignored.'
];

for (const context of ['app', 'project'] as const) {
  for (const theme of ['dark', 'light'] as const) {
    test(`keeps the complete ${context} answer after completion and follow-ups in ${theme} mode`, async ({ session }, testInfo) => {
      if (context === 'project') await session.openEditor();
      await session.openAgent();
      const { page } = session;
      await page.evaluate(({ theme, context }) => {
        const PM = (window as any).PM;
        PM.theme.apply(theme);
        if (context === 'project') PM.AgentUI.setAccess('project');
        (window as any).__preservationRuns = [];
        PM.CodexBridge.request = (_prompt: unknown, _schema: unknown, _images: unknown, options: any) => new Promise(resolve => {
          (window as any).__preservationRuns.push({ options, resolve });
        });
      }, { theme, context });
      const composer = page.getByRole('textbox', { name: 'Message Powermove agent', exact: true });
      await composer.fill('where is the artifacts folder');
      await composer.press('Enter');
      await session.openAgent();
      await expect.poll(() => page.evaluate(() => (window as any).__preservationRuns.length)).toBe(1);
      await page.evaluate(text => {
        const run = (window as any).__preservationRuns[0];
        run.options.onTrace({ kind: 'tool-start', itemId: 'read', toolName: 'bash', label: 'Read artifacts' });
        run.options.onTrace({ kind: 'tool-end', itemId: 'read', toolName: 'bash', isError: false });
        // Exercise incremental chunks and multiple paragraphs in the same row.
        for (const paragraph of text) run.options.onTrace({ kind: 'answer', text: `${paragraph}\n\n` });
      }, paragraphs);
      const live = page.locator('.agent-trace.is-live .agent-trace-prose');
      await expect(live.locator('p')).toHaveCount(paragraphs.length);
      const before = await live.innerText();
      await page.evaluate(summary => (window as any).__preservationRuns[0].resolve({
        text: JSON.stringify({ summary, commands: [], artifacts: [], externalActions: [], notes: [] }), extensions: []
      }), paragraphs[0]);
      await expect.poll(() => page.evaluate(() => (window as any).PM.AgentUI.state.legacyPhase)).toBe('result');
      const response = page.locator('.agent-trace.is-archived .agent-trace-prose').first();
      await expect(response.locator('p')).toHaveCount(paragraphs.length);
      expect(await response.innerText()).toBe(before);

      for (const width of [240, 320, 480]) {
        await page.locator('#agent-popover').evaluate((element, width) => { (element as HTMLElement).style.width = `${width}px`; }, width);
        for (const paragraph of await response.locator('p').all()) {
          await paragraph.scrollIntoViewIfNeeded();
          await expect(paragraph).toBeInViewport();
        }
        const bounds = await response.evaluate(element => {
          const container = element.closest('.agent-trace')!.getBoundingClientRect();
          const last = element.lastElementChild!.getBoundingClientRect();
          return { contained: last.bottom <= container.bottom + 1, width: element.clientWidth, scroll: element.scrollWidth };
        });
        expect(bounds.contained).toBe(true);
        expect(bounds.scroll).toBeLessThanOrEqual(bounds.width + 1);
      }
      for (const prompt of ['ok', 'One more question']) {
        await composer.fill(prompt);
        await composer.press('Enter');
        await session.openAgent();
        await expect.poll(() => page.evaluate(() => (window as any).__preservationRuns.length)).toBe(prompt === 'ok' ? 2 : 3);
        expect(await response.innerText()).toBe(before);
        await response.locator('p').last().scrollIntoViewIfNeeded();
        await expect(response.locator('p').last()).toBeInViewport();
        await page.evaluate(() => (window as any).__preservationRuns.at(-1).resolve({
          text: JSON.stringify({ summary: 'Ready.', commands: [], artifacts: [], externalActions: [], notes: [] }), extensions: []
        }));
        await expect.poll(() => page.evaluate(() => (window as any).PM.AgentUI.state.legacyPhase)).toBe('result');
      }
      const threadId = await page.evaluate(() => (window as any).PM.AgentUI.state.threadId);
      await page.evaluate(() => (window as any).PM.AgentUI.newThread());
      await page.evaluate(id => (window as any).PM.AgentUI.switchThread(id), threadId);
      expect(await response.innerText()).toBe(before);
      await response.locator('p').last().scrollIntoViewIfNeeded();
      await page.locator('#panel-agent').screenshot({ path: testInfo.outputPath('reply-after-follow-ups.png') });
      expect(session.diagnostics.pageErrors).toEqual([]);
    });
  }
}

for (const context of ['app', 'project'] as const) {
  test(`keeps ${context} result notes after sending a message and reopening history`, async ({ session }) => {
    if (context === 'project') await session.openEditor();
    await session.openAgent();
    const { page } = session;
    await page.evaluate(context => {
      const PM = (window as any).PM;
      if (context === 'project') PM.AgentUI.setAccess('project');
      PM.CodexBridge.request = async () => ({
        text: JSON.stringify({
          summary: 'The files are in `artifacts/output/`.', commands: [], artifacts: [], externalActions: [],
          notes: ['Subfolders: logos, harnesses, fonts, and screens.', 'I could not open Finder for you.', 'The other empty folder can be ignored.']
        }), extensions: []
      });
    }, context);
    const composer = page.getByRole('textbox', { name: 'Message Powermove agent', exact: true });
    await composer.fill('where is the artifacts folder');
    await composer.press('Enter');
    await session.openAgent();
    await expect.poll(() => page.evaluate(() => (window as any).PM.AgentUI.state.legacyPhase)).toBe('result');
    const log = page.getByRole('log', { name: 'Agent conversation' });
    await expect(log).toContainText('The other empty folder can be ignored.');
    await expect(log.getByText('I could not open Finder for you.', { exact: true })).toHaveCount(1);
    await page.evaluate(() => { (window as any).PM.CodexBridge.request = () => new Promise(() => {}); });
    await composer.fill('ok');
    await composer.press('Enter');
    await session.openAgent();
    await expect(log).toContainText('Subfolders: logos, harnesses, fonts, and screens.');
    await expect(log).toContainText('I could not open Finder for you.');
    await expect(log).toContainText('The other empty folder can be ignored.');
    await page.getByRole('button', { name: 'Stop current run', exact: true }).click();
    const threadId = await page.evaluate(() => (window as any).PM.AgentUI.state.threadId);
    await page.evaluate(() => (window as any).PM.AgentUI.newThread());
    await page.evaluate(id => (window as any).PM.AgentUI.switchThread(id), threadId);
    await expect(log).toContainText('Subfolders: logos, harnesses, fonts, and screens.');
    await expect(log).toContainText('I could not open Finder for you.');
    await expect(log).toContainText('The other empty folder can be ignored.');
    if (context === 'app') {
      await session.relaunch();
      await session.openAgent();
      const restoredLog = session.page.getByRole('log', { name: 'Agent conversation' });
      await expect.poll(() => session.page.evaluate(() => (window as any).PM.AgentUI.state.threadId)).toBe(threadId);
      await expect(restoredLog).toContainText('Subfolders: logos, harnesses, fonts, and screens.');
      await expect(restoredLog).toContainText('I could not open Finder for you.');
      await expect(restoredLog).toContainText('The other empty folder can be ignored.');
    }
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
}
