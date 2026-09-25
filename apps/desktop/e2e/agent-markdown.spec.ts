import path from 'node:path';
import { test, expect, repoRoot } from './helpers/app';

const binary = path.join(repoRoot, 'src/main/claude/__fixtures__/fake-claude-markdown.sh');

for (const mode of ['markdown-stream', 'markdown-summary']) {
  test.describe(mode, () => {
    test.use({ desktopLaunchOptions: { env: { CLAUDE_BINARY: binary, FAKE_CLAUDE_MODE: mode } } });

    test('formats Claude output through the CLI, IPC, and agent conversation', async ({ session }, testInfo) => {
      await session.openEditor();
      const page = session.page;
      await page.evaluate(() => {
        const PM = (window as any).PM;
        PM.SpatialAssistant.open();
        PM.AgentUI.setProvider('claude');
        PM.AgentUI.setAccess('project');
        PM.AgentHarness.observe = async () => ({ state: {}, times: [], images: [] });
      });
      await page.getByRole('textbox', { name: 'Message Powermove agent', exact: true }).fill('Show a formatted reply');
      await page.getByRole('button', { name: 'Send message', exact: true }).click();
      const reply = page.locator(mode === 'markdown-stream'
        ? '.agent-trace.is-archived > .agent-trace-prose' : '.agent-msg.assistant .agent-reply');
      await expect(reply.locator('.agent-md-h')).toHaveText('Complete');
      await expect(reply.locator('.is-bold')).toHaveText('hello');
      await expect(reply.locator('.agent-md-li')).toHaveCount(2);
      await expect(reply.locator('pre code')).toHaveText('const ready = true;');
      await expect(reply).toContainText('Literal: **example**');
      await expect(reply).not.toContainText('\\*');
      await expect(reply).not.toContainText('**hello**');
      expect(await reply.locator('.is-bold').evaluate(element => Number(getComputedStyle(element).fontWeight))).toBeGreaterThanOrEqual(600);
      await reply.scrollIntoViewIfNeeded();
      await page.locator('#panel-agent').screenshot({ path: testInfo.outputPath('formatted-reply.png') });
      expect(session.diagnostics.pageErrors).toEqual([]);
    });
  });
}

test('formats progress and preserves Markdown in a completed long streamed reply', async ({ session }, testInfo) => {
  await session.openEditor();
  const page = session.page;
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.SpatialAssistant.open();
    PM.AgentUI.setAccess('project');
    PM.AgentHarness.observe = async () => ({ state: {}, times: [], images: [] });
    PM.CodexBridge.request = async (_prompt: unknown, _schema: unknown, _images: unknown, options: any) => {
      options.onProgress('**Preparing** the `reply`');
      return new Promise(resolve => {
        (window as any).__finishMarkdownReply = () => {
          const summary = `## Complete\n\n**${'word '.repeat(1300).trim()}**\n\n- First\n- Second`;
          for (let offset = 0; offset < summary.length; offset += 2000) {
            options.onTrace({ kind: 'answer', text: summary.slice(offset, offset + 2000) });
          }
          resolve({ text: JSON.stringify({ summary,
            commands: [], artifacts: [], externalActions: [], notes: [], extensions: []
          }) });
        };
      });
    };
  });
  await page.getByRole('textbox', { name: 'Message Powermove agent', exact: true }).fill('Show progress and a reply');
  await page.getByRole('button', { name: 'Send message', exact: true }).click();
  const progress = page.locator('.agent-trace-loading');
  await expect(progress.locator('.is-bold')).toHaveText('Preparing');
  await expect(progress.locator('.agent-trace-code')).toHaveText('reply');
  await expect(progress).not.toContainText('**');
  await page.locator('#panel-agent').screenshot({ path: testInfo.outputPath('formatted-progress.png') });
  await page.evaluate(() => (window as any).__finishMarkdownReply());
  const reply = page.locator('.agent-trace.is-archived > .agent-trace-prose');
  await expect(reply.locator('.agent-md-h')).toHaveText('Complete');
  await expect(reply.locator('.is-bold').filter({ hasText: 'word' })).toHaveCount(1300);
  await expect(reply.locator('.agent-md-li')).toHaveCount(2);
  await expect(progress).toHaveCount(0);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
