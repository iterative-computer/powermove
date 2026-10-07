import { mkdir } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { test, expect } from './helpers/app';

test('model-created threads preserve the visible chat and draft, deduplicate launches, and expose native thread controls', async ({ session }) => {
  await session.openEditor(); await session.openAgent();
  const { page } = session;
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.AgentUI.setAccess('project');
    PM.AgentHarness.observe = async () => ({ state: {}, times: [], images: [] });
    (window as any).__threadRequests = [];
    PM.CodexBridge.request = async (prompt: string, _schema: unknown, _images: unknown, options: any) => {
      const id = `controlled-run-${(window as any).__threadRequests.length + 1}`;
      const entry = { id, prompt, model: options.model, provider: options.provider, effort: options.reasoningEffort, stopped: false };
      (window as any).__threadRequests.push(entry);
      options.onStart?.(id);
      return new Promise((resolve, reject) => {
        (window as any).__finishThread = () => resolve({ text: JSON.stringify({ summary: 'The separate review is finished.', commands: [], artifacts: [], extensions: [], notes: [], externalActions: [] }) });
        options.signal.addEventListener('abort', () => { entry.stopped = true; const error = new Error('Stopped'); error.name = 'AbortError'; reject(error); });
      });
    };
    PM.CodexBridge.steer = async () => false;
  });
  const composer = page.getByRole('textbox', { name: 'Message Powermove agent', exact: true });
  await composer.fill('Keep this draft');
  const parent = await page.evaluate(() => (window as any).PM.AgentUI.state.threadId);
  const launched = await page.evaluate(async () => {
    const PM = (window as any).PM;
    const args = { tool: 'thread_launch', projectId: PM.proj.id, title: 'Separate review', message: 'Review the timing in this thread.', provider: 'claude', model: 'claude-sonnet-5-5', reasoningEffort: 'medium', access: 'project', clientRequestId: 'launch-review', callerThreadId: PM.AgentUI.state.threadId };
    const [a, b] = await Promise.all([PM.SpatialAssistant.controlThread(args), PM.SpatialAssistant.controlThread(args)]);
    return { a, b, requests: (window as any).__threadRequests.length };
  });
  expect(launched.a.threadId).toBe(launched.b.threadId); expect(launched.requests).toBe(1);
  expect(await page.evaluate(() => (window as any).PM.AgentUI.state.threadId)).toBe(parent);
  await expect(composer).toHaveText('Keep this draft');
  const picker = page.getByRole('button', { name: 'Switch thread', exact: true });
  await expect(picker.locator('.thread-running')).toHaveText('1');
  await picker.click();
  const option = page.getByRole('option').filter({ hasText: 'Separate review' });
  await expect(option).toContainText('Working…');
  const visuals = process.env.POWERMOVE_AGENT_ARTIFACTS || path.join(os.tmpdir(), 'powermove-thread-controls-visuals'); await mkdir(visuals, { recursive: true });
  for (const theme of ['dark', 'light']) {
    await page.evaluate(theme => (window as any).PM.theme.apply(theme), theme);
    await expect.poll(() => page.locator('#panel-agent').evaluate(element => element.getAnimations({ subtree: true }).filter(animation => Number.isFinite(animation.effect?.getTiming().iterations ?? 1)).every(animation => animation.playState === 'finished'))).toBe(true);
    await page.screenshot({ path: path.join(visuals, `orchestrated-threads-${theme}.png`) });
  }
  await option.click();
  await expect(page.getByRole('log')).toContainText('Review the timing in this thread.');
  const sent = await page.evaluate(async threadId => {
    const PM = (window as any).PM;
    const args = { tool: 'thread_send', projectId: PM.proj.id, threadId, message: 'Now focus on easing.', provider: 'chatgpt', model: 'gpt-5.6-sol', reasoningEffort: 'high', access: 'project', reconfigure: true, callerThreadId: 'parent', clientRequestId: 'easing-direction' };
    const a = await PM.SpatialAssistant.controlThread(args), b = await PM.SpatialAssistant.controlThread(args);
    return { a, b, requests: (window as any).__threadRequests };
  }, launched.a.threadId);
  expect(sent.a.runId).not.toBe(launched.a.runId); expect(sent.b.delivery).toBe('already_sent');
  expect(sent.requests).toHaveLength(2); expect(sent.requests[0].stopped).toBe(true);
  expect(sent.requests[1]).toMatchObject({ provider: 'chatgpt', model: 'gpt-5.6-sol', effort: 'high' });
  expect(sent.requests[1].prompt).toContain('Review the timing in this thread.');
  await expect(page.getByRole('log')).toContainText('Now focus on easing.');
  await page.evaluate(() => (window as any).__finishThread());
  await expect(page.getByRole('log')).toContainText('The separate review is finished.');
  const read = await page.evaluate(threadId => {
    const PM = (window as any).PM;
    return PM.SpatialAssistant.controlThread({ tool: 'thread_read', projectId: PM.proj.id, threadId, offset: 0, limit: 30 });
  }, launched.a.threadId);
  expect(read.thread).toMatchObject({ busy: false, status: 'completed', model: 'gpt-5.6-sol', reasoningEffort: 'high' });
  expect(read.messages.filter((message: any) => message.text === 'Now focus on easing.')).toHaveLength(1);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('a launched planning thread keeps inspection access and can be stopped from its normal composer', async ({ session }) => {
  await session.openEditor(); await session.openAgent();
  const { page } = session;
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.AgentHarness.observe = async () => ({ state: {}, times: [], images: [] });
    PM.CodexBridge.request = async (_prompt: string, _schema: unknown, _images: unknown, options: any) => {
      options.onStart?.('planning-run');
      (window as any).__planningOptions = { mode: options.mode ?? 'editor', access: options.access ?? 'editor' };
      return new Promise((_resolve, reject) => options.signal.addEventListener('abort', () => { const error = new Error('Stopped'); error.name = 'AbortError'; reject(error); }));
    };
  });
  const launched = await page.evaluate(() => {
    const PM = (window as any).PM;
    return PM.SpatialAssistant.controlThread({ tool: 'thread_launch', projectId: PM.proj.id, title: 'Planning review', message: 'Plan a timing update.', provider: 'chatgpt', model: 'gpt-5.6-sol', reasoningEffort: 'medium', access: 'editor' });
  });
  expect(await page.evaluate(() => (window as any).__planningOptions)).toEqual({ mode: 'editor', access: 'editor' });
  await page.getByRole('button', { name: 'Switch thread', exact: true }).click();
  await page.getByRole('option').filter({ hasText: 'Planning review' }).click();
  await page.getByRole('button', { name: 'Stop current run', exact: true }).click();
  const read = await page.evaluate(threadId => {
    const PM = (window as any).PM;
    return PM.SpatialAssistant.controlThread({ tool: 'thread_read', projectId: PM.proj.id, threadId, offset: 0, limit: 30 });
  }, launched.threadId);
  expect(read.thread).toMatchObject({ busy: false, status: 'cancelled', access: 'editor' });
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('the parent can read and answer a blocked background thread without interrupting it', async ({ session }) => {
  await session.openEditor(); await session.openAgent();
  const { page } = session;
  const parent = await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.AgentHarness.observe = async () => ({ state: {}, times: [], images: [] });
    (window as any).__questionRunCount = 0;
    (window as any).webkit.messageHandlers.pmCodex.postMessage = (request: any) => {
      (window as any).__questionRunCount++;
      (window as any).__questionRun = request;
      queueMicrotask(() => PM.CodexBridge.trace(request.id, { kind: 'question', itemId: 'timing-question', transport: 'reply', blocking: true,
        questions: [{ id: 'timing', question: 'Which duration should I use?', header: 'Duration', secret: false, options: [] }] }));
    };
    (window as any).webkit.messageHandlers.pmCodexAnswer.postMessage = (request: any) => {
      (window as any).__questionAnswer = request;
      queueMicrotask(() => PM.CodexBridge.resolveSteer(request.replyId, true));
    };
    return PM.AgentUI.state.threadId;
  });
  const launched = await page.evaluate(() => {
    const PM = (window as any).PM;
    return PM.SpatialAssistant.controlThread({ tool: 'thread_launch', projectId: PM.proj.id, title: 'Timing question', message: 'Review timing.', provider: 'chatgpt', model: 'gpt-5.6-sol', reasoningEffort: 'medium', access: 'project' });
  });
  const read = await page.evaluate(threadId => {
    const PM = (window as any).PM;
    return PM.SpatialAssistant.controlThread({ tool: 'thread_read', projectId: PM.proj.id, threadId, offset: 0, limit: 10 });
  }, launched.threadId);
  expect(read.questions).toMatchObject([{ itemId: 'timing-question', questions: [{ question: 'Which duration should I use?' }] }]);
  const reply = await page.evaluate(threadId => {
    const PM = (window as any).PM;
    return PM.SpatialAssistant.controlThread({ tool: 'thread_send', projectId: PM.proj.id, threadId, message: 'Use 3 seconds.', provider: 'chatgpt', model: 'gpt-5.6-sol', reasoningEffort: 'medium', access: 'project' });
  }, launched.threadId);
  expect(reply).toMatchObject({ delivery: 'answered', runId: launched.runId });
  expect(await page.evaluate(() => (window as any).__questionAnswer.answers)).toEqual({ timing: ['Use 3 seconds.'] });
  expect(await page.evaluate(() => (window as any).__questionRunCount)).toBe(1);
  expect(await page.evaluate(() => (window as any).PM.AgentUI.state.threadId)).toBe(parent);
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.CodexBridge.resolve((window as any).__questionRun.id, { ok: true, dataBase64: btoa(JSON.stringify({ summary: 'Reviewed the 3-second timing.', commands: [], artifacts: [], notes: [], extensions: [], externalActions: [] })) });
  });
  await expect.poll(() => page.evaluate(threadId => {
    const PM = (window as any).PM;
    return PM.SpatialAssistant.controlThread({ tool: 'thread_read', projectId: PM.proj.id, threadId, offset: 0, limit: 10 }).then((state: any) => state.thread.busy);
  }, launched.threadId)).toBe(false);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
