import { mkdir } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { test, expect, expandAgentWork } from './helpers/app';

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
  await expect(panel.getByText('Claude Sonnet 5.5', { exact: true })).toBeVisible();
  await expect(panel.getByText('Checking the timing', { exact: true })).toBeVisible();
  await expect(panel.locator('.agent-subagents-summary')).toHaveText('1 working · 1 done');
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
  await expect(panel.locator('.agent-subagents-summary')).toHaveText('1 done · 1 failed');
  await expect.poll(() => panel.locator('li[data-phase="failed"] .agent-subagent-what').evaluate(element => element.getBoundingClientRect().height > 0 && getComputedStyle(element).opacity === '1')).toBe(true);
  await page.screenshot({ path: path.join(visuals, 'subagents-light.png') });
  expect(await panel.evaluate(element => element.scrollWidth <= element.clientWidth)).toBe(true);
  await page.evaluate(() => (window as any).__finishSubagents({ text: JSON.stringify({ summary: 'Review received.', commands: [], artifacts: [], notes: [], extensions: [], externalActions: [] }) }));
  await expect(panel.getByText('Review received.', { exact: true })).toBeVisible();
  await expect(panel.getByText(/Worked for .* · 2 subagents, 1 failed/)).toBeVisible();
  await expandAgentWork(page);
  await expect(panel.getByText('Claude Sonnet 5.5', { exact: true })).toBeVisible();
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('each subagent works in a sub-thread under its parent that the person can open, message and leave', async ({ session }) => {
  await session.openEditor(); await session.openAgent();
  const { page, app } = session;
  await app.evaluate(({ ipcMain }) => {
    (globalThis as any).__taskMessages = [];
    ipcMain.removeHandler('codex:message-task');
    ipcMain.handle('codex:message-task', (_event, input) => { (globalThis as any).__taskMessages.push(input); return 'live'; });
  });
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.AgentUI.setAccess('project');
    PM.AgentHarness.observe = async () => ({ state: {}, times: [], images: [] });
    PM.CodexBridge.request = async (_prompt: unknown, _schema: unknown, _images: unknown, options: any) => {
      (window as any).__trace = options.onTrace;
      return new Promise(resolve => { (window as any).__finish = resolve; });
    };
  });
  await page.getByRole('textbox', { name: 'Message Powermove agent', exact: true }).fill('Polish the title.');
  await page.getByRole('button', { name: 'Send message', exact: true }).click(); await session.openAgent();
  await expect.poll(() => page.evaluate(() => typeof (window as any).__trace)).toBe('function');
  const parent = await page.evaluate(() => (window as any).PM.AgentUI.state.threadId);
  await page.evaluate(() => {
    const trace = (window as any).__trace;
    const task = {
      taskId: 'timing-task', parentThreadId: 'parent', childThreadId: 'child', childRunId: 'child-run', parentTaskId: null,
      requestId: 'root-run', title: 'Tighten the timing', providerInstanceId: 'claude', model: 'claude-sonnet-5-5',
      status: 'running', workState: 'working', hasPendingChildRuns: false, summary: null, progress: 'Reading keyframes', startedAt: Date.now(), waitTimedOut: false
    };
    (window as any).__task = task;
    trace({ kind: 'task', task });
    trace({ kind: 'subagent', taskId: 'timing-task', step: { kind: 'prompt', text: 'Tighten the title timing to 12 frames.' } });
    trace({ kind: 'subagent', taskId: 'timing-task', step: { kind: 'tool-start', itemId: 'read-1', toolName: 'read_file', label: 'Read', detail: 'title.json' } });
    trace({ kind: 'subagent', taskId: 'timing-task', step: { kind: 'tool-end', itemId: 'read-1', isError: false, output: 'ok' } });
  });
  const panel = page.locator('#panel-agent');
  // The parent keeps one tidy card; nothing from the subagent spills into it.
  await expect(panel.getByRole('log')).not.toContainText('Tighten the title timing to 12 frames.');
  const picker = page.getByRole('button', { name: 'Switch thread', exact: true });
  await expect(picker.locator('.thread-running')).toHaveCount(0);
  await picker.click();
  const sub = page.locator('.thread-row.sub').filter({ hasText: 'Tighten the timing' });
  await expect(sub).toContainText('Subagent · Working…');
  await page.keyboard.press('Escape');

  await panel.getByRole('button', { name: 'Open Tighten the timing' }).click();
  await expect.poll(() => page.evaluate(() => (window as any).PM.AgentUI.state.threadId)).toBe('sub-timing-task');
  await expect(picker).toContainText('Tighten the timing');
  await expect(panel.locator('.subthread-bar')).toContainText('A subagent is working here for this thread.');
  await expect(panel.getByRole('log')).toContainText('Tighten the title timing to 12 frames.');
  await expect(panel.getByRole('log')).toContainText('title.json');

  await page.evaluate(() => (window as any).__trace({ kind: 'subagent', taskId: 'timing-task', step: { kind: 'answer', text: 'Trimmed the title to 12 frames.' } }));
  await expect(panel.getByRole('log')).toContainText('Trimmed the title to 12 frames.');
  const composer = page.getByRole('textbox', { name: 'Message Powermove agent', exact: true });
  await composer.fill('Keep the fade-out.');
  await page.getByRole('button', { name: 'Steer current run', exact: true }).click();
  await expect.poll(() => app.evaluate(() => (globalThis as any).__taskMessages)).toEqual([{ requestId: 'root-run', taskId: 'timing-task', message: 'Keep the fade-out.' }]);
  await expect(panel.getByRole('log')).toContainText('Keep the fade-out.');

  await page.evaluate(() => (window as any).__trace({ kind: 'task', task: { ...(window as any).__task, status: 'completed', workState: 'result_available', summary: 'Trimmed the title to 12 frames.', endedAt: Date.now() } }));
  await expect(panel.locator('.subthread-bar')).toContainText('A subagent worked here for this thread.');
  await expect(page.getByRole('button', { name: 'Steer current run', exact: true })).toHaveCount(0);
  await expect(panel.getByText('Trimmed the title to 12 frames.', { exact: true })).toHaveCount(1);

  await panel.getByRole('button', { name: /^Back to / }).click();
  await expect.poll(() => page.evaluate(() => (window as any).PM.AgentUI.state.threadId)).toBe(parent);
  await expect(panel.locator('.subthread-bar')).toHaveCount(0);
  // The sub-thread survives a reload of the saved history, still nested.
  const saved = await page.evaluate(() => {
    const PM = (window as any).PM;
    const archive = PM.store.get(`agentThreads.${PM.proj.id}`, null);
    return archive.threads.find((thread: any) => thread.id === 'sub-timing-task');
  });
  expect(saved).toMatchObject({ parentId: parent, subagent: { taskId: 'timing-task', status: 'completed' } });
  await page.evaluate(() => (window as any).__finish({ text: JSON.stringify({ summary: 'Title polished.', commands: [], artifacts: [], notes: [], extensions: [], externalActions: [] }) }));
  await expect(panel.getByText('Title polished.', { exact: true })).toBeVisible();
  expect(session.diagnostics.pageErrors).toEqual([]);
});
