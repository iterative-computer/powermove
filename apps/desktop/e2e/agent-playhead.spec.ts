import path from 'node:path';
import { expect, test, repoRoot } from './helpers/app';

test.use({ desktopLaunchOptions: { env: { CODEX_BINARY: path.join(repoRoot, 'src/main/codex/__fixtures__/fake-codex-app-server.sh') } } });

test('sending messages preserves the paused playhead without capturing preview frames', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.pause();
    PM.setTime(1.5);
    PM.AgentUI.setAccess('editor');
    (window as any).__sendTimes = [];
    (window as any).__agentCaptures = 0;
    const capture = PM.Export.snapshotAsync;
    PM.Export.snapshotAsync = (...args: any[]) => { (window as any).__agentCaptures++; return capture(...args); };
    PM.bus.on('time', (time: number) => (window as any).__sendTimes.push(time));
    PM.CodexBridge.request = async (_prompt: any, _schema: any, images: any[], options: any) => {
      (window as any).__requestImages = images;
      return options.mode === 'autonomous'
        ? { text: JSON.stringify({ summary: 'Hello', commands: [], artifacts: [], externalActions: [], notes: [] }), extensions: [] }
        : JSON.stringify({ operation: 'noop', message: 'Hello' });
    };
  });
  const composer = page.getByRole('textbox', { name: 'Message Powermove agent', exact: true });
  for (const access of ['editor', 'project']) {
    await page.evaluate(access => (window as any).PM.AgentUI.setAccess(access), access);
    for (const method of ['click', 'enter']) {
      await composer.fill('Hello');
      if (method === 'click') await page.getByRole('button', { name: 'Send message', exact: true }).click();
      else await composer.press('Enter');
      await expect.poll(() => page.evaluate(() => (window as any).PM.AgentUI.state.legacyPhase)).toBe(access === 'editor' ? 'conversation' : 'result');
      expect(await page.evaluate(() => ({ time: (window as any).PM.time, playing: (window as any).PM.playing, events: (window as any).__sendTimes }))).toEqual({ time: 1.5, playing: false, events: [] });
      expect(await page.evaluate(() => (window as any).__agentCaptures)).toBe(0);
      expect(await page.evaluate(() => (window as any).__requestImages)).toEqual([]);
    }
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('thinking remains visible when steering falls back and another message is sent', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.AgentUI.setAccess('project');
    (window as any).__traceRuns = [];
    PM.CodexBridge.request = (_prompt: any, _schema: any, _images: any, options: any) => new Promise(resolve => {
      (window as any).__traceRuns.push({ options, resolve });
    });
    PM.CodexBridge.steer = () => new Promise(resolve => { (window as any).__finishSteer = resolve; });
  });
  const composer = page.getByRole('textbox', { name: 'Message Powermove agent', exact: true });
  await composer.fill('Inspect the composition');
  await composer.press('Enter');
  await expect.poll(() => page.evaluate(() => (window as any).__traceRuns.length)).toBe(1);
  await page.evaluate(() => (window as any).__traceRuns[0].options.onTrace({ kind: 'thought', text: 'Inspecting the composition' }));
  await composer.fill('Focus on timing');
  await composer.press('Enter');
  await page.waitForFunction(() => typeof (window as any).__finishSteer === 'function');
  await page.evaluate(() => {
    (window as any).__traceRuns[0].options.onTrace({ kind: 'thought', text: 'Checking the timing' });
    (window as any).__finishSteer(false);
  });
  await expect.poll(() => page.evaluate(() => (window as any).__traceRuns.length)).toBe(2);
  const archived = page.locator('.agent-trace.is-archived');
  await expect(archived).toHaveCount(2);
  await expect(archived.nth(0)).toContainText('Inspecting the composition');
  await expect(archived.nth(1)).toContainText('Checking the timing');
  await page.evaluate(() => (window as any).__traceRuns[1].resolve({ text: JSON.stringify({ summary: 'Done', commands: [], artifacts: [], externalActions: [], notes: [] }), extensions: [] }));
  await expect.poll(() => page.evaluate(() => (window as any).PM.AgentUI.state.legacyPhase)).toBe('result');
  // A provider can deliver a final reasoning fragment after its result.
  await page.evaluate(() => (window as any).__traceRuns[1].options.onTrace({ kind: 'thought', text: 'Finished checking' }));
  await composer.fill('Continue');
  await composer.press('Enter');
  await expect.poll(() => page.evaluate(() => (window as any).__traceRuns.length)).toBe(3);
  await expect(archived).toHaveCount(3);
  await expect(archived.nth(2)).toContainText('Finished checking');
  await page.evaluate(() => (window as any).PM.AgentUI.stop());
  expect(session.diagnostics.pageErrors).toEqual([]);
});
