import { mkdir } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { test, expect } from './helpers/app';

test('subagents update in place, expose results and Stop, and fit native light and dark panels', async ({ session }) => {
  await session.openEditor(); await session.openAgent();
  const { page, app } = session;
  await page.setViewportSize({ width: 1440, height: 900 });
  await app.evaluate(({ ipcMain }) => {
    (globalThis as any).__stoppedTasks = [];
    ipcMain.removeHandler('codex:cancel-task');
    ipcMain.handle('codex:cancel-task', (_event, input) => { (globalThis as any).__stoppedTasks.push(input); });
  });
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.theme.apply('dark');
    PM.AgentUI.setAccess('project');
    PM.AgentHarness.observe = async () => ({ state: {}, times: [], images: [] });
    PM.CodexBridge.request = async (_prompt: unknown, _schema: unknown, _images: unknown, options: any) => {
      (window as any).__subagentTrace = options.onTrace;
      (window as any).__finishSubagents = null;
      return new Promise(resolve => { (window as any).__finishSubagents = resolve; });
    };
  });
  await page.getByRole('textbox', { name: 'Message Powermove agent', exact: true }).fill('Have Claude review the title animation.');
  await page.getByRole('button', { name: 'Send message', exact: true }).click(); await session.openAgent();
  await expect.poll(() => page.evaluate(() => typeof (window as any).__subagentTrace)).toBe('function');
  await page.evaluate(() => {
    const task = {
      taskId: 'review-task', parentThreadId: 'parent', childThreadId: 'child', childRunId: 'child-run', parentTaskId: null,
      requestId: 'root-run', title: 'Review the title animation', providerInstanceId: 'claude', model: 'claude-sonnet-5-5',
      status: 'running', workState: 'working', hasPendingChildRuns: false, summary: null, progress: 'Checking the timing', startedAt: Date.now(), waitTimedOut: false
    };
    (window as any).__reviewTask = task;
    (window as any).__subagentTrace({ kind: 'task', task });
    (window as any).__subagentTrace({ kind: 'task', task: { ...task, taskId: 'nested-task', childRunId: 'nested-run', parentTaskId: 'review-task', title: 'Check easing', providerInstanceId: 'compatible', model: 'Local model', status: 'completed', workState: 'result_available', summary: 'Easing looks consistent.', endedAt: Date.now() } });
  });
  const panel = page.locator('#panel-agent');
  await expect(panel.getByText('Claude · Sonnet 5.5', { exact: true })).toBeVisible();
  await expect(panel.getByRole('button', { name: 'Stop Review the title animation' })).toBeVisible();
  await panel.getByRole('button', { name: 'Stop Review the title animation' }).click();
  await expect.poll(() => app.evaluate(() => (globalThis as any).__stoppedTasks)).toEqual([{ requestId: 'root-run', taskId: 'review-task' }]);
  const visuals = process.env.POWERMOVE_AGENT_ARTIFACTS || path.join(os.tmpdir(), 'powermove-subagents-visuals'); await mkdir(visuals, { recursive: true });
  await page.screenshot({ path: path.join(visuals, 'subagents-dark.png') });
  await page.evaluate(() => (window as any).__subagentTrace({ kind: 'task', task: { ...(window as any).__reviewTask, status: 'failed', workState: 'result_available', summary: 'The connection closed before the review finished.', endedAt: Date.now() } }));
  await expect(panel.getByText('The connection closed before the review finished.', { exact: true })).toBeVisible();
  await expect(panel.getByText('Review the title animation', { exact: true })).toHaveCount(1);
  await expect(panel.getByRole('button', { name: 'Stop Review the title animation' })).toHaveCount(0);
  await page.evaluate(() => (window as any).PM.theme.apply('light'));
  await expect.poll(() => panel.locator('.is-failed .agent-tool-output').evaluate(element => element.getBoundingClientRect().height > 0 && getComputedStyle(element).opacity === '1' && element.getAnimations({ subtree: true }).every(animation => animation.playState === 'finished'))).toBe(true);
  await page.screenshot({ path: path.join(visuals, 'subagents-light.png') });
  expect(await panel.evaluate(element => element.scrollWidth <= element.clientWidth)).toBe(true);
  await page.evaluate(() => (window as any).__finishSubagents({ text: JSON.stringify({ summary: 'Review received.', commands: [], artifacts: [], notes: [], extensions: [], externalActions: [] }) }));
  await expect(panel.getByText('Review received.', { exact: true })).toBeVisible();
  await expect(panel.getByText('Claude · Sonnet 5.5', { exact: true })).toBeVisible();
  expect(session.diagnostics.pageErrors).toEqual([]);
});
