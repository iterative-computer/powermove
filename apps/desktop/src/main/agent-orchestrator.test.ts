import { mkdtemp, readdir, rm } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { AgentTask, OrchestrationProvider } from '../shared/agent-orchestration';
import type { CodexRunRequest, CodexRunResult, CodexTraceEvent } from '../shared/ipc';
import { AgentOrchestrator, type AgentOrchestratorOptions } from './agent-orchestrator';

const providers: OrchestrationProvider[] = (['chatgpt', 'claude', 'compatible'] as const).map(providerInstanceId => ({
  providerInstanceId, driverKind: providerInstanceId, displayName: providerInstanceId,
  canRunChildTask: true, canRunCrossProviderChildTask: true, constraints: [],
  models: [{ id: `${providerInstanceId}-model`, label: `${providerInstanceId} model`, options: [{
    id: 'reasoningEffort', label: 'Reasoning', type: 'select', options: [{ id: 'medium', label: 'Medium' }]
  }] }]
}));
const request = (patch: Partial<CodexRunRequest> = {}): CodexRunRequest => ({
  id: 'root-run', threadId: 'root-thread', provider: 'chatgpt', mode: 'autonomous', access: 'project',
  projectId: 'project', projectName: 'Project', projectJSON: '{"revision":1}', prompt: 'Original user task',
  schema: null, images: [], attachments: [], consentToken: null, model: 'chatgpt-model', reasoningEffort: 'medium', ...patch
});
const ok = (text: string): CodexRunResult => ({ ok: true, text, access: 'project' });
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(done => { resolve = done; });
  return { promise, resolve };
}
const directories: string[] = [];
async function harness(execute: AgentOrchestratorOptions['execute'], cancel = vi.fn(async (_id: string) => {})) {
  const directory = await mkdtemp(path.join(os.tmpdir(), 'pm-subagents-')); directories.push(directory);
  const trace: CodexTraceEvent[] = [];
  const manager = new AgentOrchestrator({ directory, providers: async () => providers, execute, cancel });
  return { manager, trace, directory, cancel, start: (req = request()) => manager.run(req, step => trace.push(step), () => {}) };
}
afterEach(async () => { await Promise.all(directories.splice(0).map(directory => rm(directory, { recursive: true, force: true }))); });

describe('app-owned provider orchestration', () => {
  it('runs Claude and compatible children in parallel and automatically resumes the parent with both results', async () => {
    const claude = deferred<CodexRunResult>(), compatible = deferred<CodexRunResult>();
    const requests: CodexRunRequest[] = [];
    let children: AgentTask[] = [];
    const execute = vi.fn<AgentOrchestratorOptions['execute']>(async (req, context) => {
      requests.push(req);
      if (context.isChild) return req.provider === 'claude' ? claude.promise : compatible.promise;
      if (context.continuation) { expect(req.prompt).toContain('Claude result'); expect(req.prompt).toContain('Local result'); return ok('Finished with both results'); }
      children = await Promise.all(['claude', 'compatible'].map(providerInstanceId => h.manager.call(req.id, 'delegate_task', {
        task: `Review using ${providerInstanceId}`, target: { providerInstanceId, options: { reasoningEffort: 'medium' } }
      }) as Promise<AgentTask>));
      return ok('Waiting for the reviews');
    });
    const h = await harness(execute);
    const run = h.start();
    await vi.waitFor(() => expect(children).toHaveLength(2));
    expect(new Set(children.map(child => child.childThreadId)).size).toBe(2);
    expect(requests.filter(req => req.id !== 'root-run').map(req => req.provider).sort()).toEqual(['claude', 'compatible']);
    expect(requests.filter(req => req.id !== 'root-run').every(req => req.projectId === 'project' && req.access === 'project' && !req.prompt.includes('Original user task'))).toBe(true);
    claude.resolve(ok('Claude result'));
    await vi.waitFor(() => expect(h.trace.some(step => step.kind === 'task' && step.task.status === 'completed')).toBe(true));
    expect(execute.mock.calls.filter(([, context]) => context.continuation)).toHaveLength(0);
    compatible.resolve(ok('Local result'));
    await expect(run).resolves.toMatchObject({ ok: true, text: 'Finished with both results' });
    expect(execute.mock.calls.filter(([, context]) => context.continuation)).toHaveLength(1);
  });

  it('a wait timeout keeps the task alive; terminal status reads acknowledge delivery', async () => {
    const child = deferred<CodexRunResult>(), root = deferred<CodexRunResult>();
    let task!: AgentTask;
    const h = await harness(async (req, context) => {
      if (context.isChild) return child.promise;
      task = await h.manager.call(req.id, 'delegate_task', { task: 'Review', mode: 'wait', timeoutMs: 0 }) as AgentTask;
      return root.promise;
    });
    const run = h.start();
    await vi.waitFor(() => expect(task?.waitTimedOut).toBe(true));
    expect(await h.manager.call('root-run', 'task_status', { taskId: task.taskId })).toMatchObject({ status: 'running', waitTimedOut: false });
    child.resolve(ok('The review is complete'));
    await vi.waitFor(async () => expect(await h.manager.call('root-run', 'task_status', { taskId: task.taskId })).toMatchObject({ status: 'completed', summary: 'The review is complete' }));
    root.resolve(ok('Already consumed the result'));
    await expect(run).resolves.toMatchObject({ text: 'Already consumed the result' });
  });

  it('deduplicates concurrent retries and rejects reuse of a key for a different brief', async () => {
    const child = deferred<CodexRunResult>(), root = deferred<CodexRunResult>();
    let results: AgentTask[] = [];
    let childCalls = 0;
    const h = await harness(async (req, context) => {
      if (context.isChild) { childCalls++; return child.promise; }
      results = await Promise.all([1, 2].map(() => h.manager.call(req.id, 'delegate_task', { task: 'One review', clientRequestId: 'round-one' }) as Promise<AgentTask>));
      return root.promise;
    });
    const run = h.start();
    await vi.waitFor(() => expect(results).toHaveLength(2));
    expect(results[0]!.taskId).toBe(results[1]!.taskId);
    expect(childCalls).toBe(1);
    await expect(h.manager.call('root-run', 'delegate_task', { task: 'Different review', clientRequestId: 'round-one' })).rejects.toThrow('different task');
    child.resolve(ok('Done'));
    await vi.waitFor(async () => expect(await h.manager.call('root-run', 'task_status', { taskId: results[0]!.taskId })).toMatchObject({ status: 'completed' }));
    root.resolve(ok('Done'));
    await run;
  });

  it('keeps a child nonterminal while its nested task is working', async () => {
    const grandchild = deferred<CodexRunResult>(), root = deferred<CodexRunResult>();
    let childTask!: AgentTask, nested!: AgentTask;
    const h = await harness(async (req, context) => {
      if (!context.isChild) {
        childTask = await h.manager.call(req.id, 'delegate_task', { task: 'Child review', target: { providerInstanceId: 'claude' } }) as AgentTask;
        return root.promise;
      }
      if (context.continuation) return ok('Child combines nested result');
      if (req.provider === 'compatible') return grandchild.promise;
      nested = await h.manager.call(req.id, 'delegate_task', { task: 'Nested review', target: { providerInstanceId: 'compatible' } }) as AgentTask;
      return ok('Waiting for nested review');
    });
    const run = h.start();
    await vi.waitFor(() => expect(nested?.taskId).toBeTruthy());
    await vi.waitFor(async () => expect(await h.manager.call('root-run', 'task_status', { taskId: childTask.taskId })).toMatchObject({ status: 'waiting', workState: 'waiting_for_children', hasPendingChildRuns: true, summary: null }));
    await expect(h.manager.call(childTask.childRunId, 'task_status', { taskId: childTask.taskId })).rejects.toThrow('does not belong');
    grandchild.resolve(ok('Nested result'));
    await vi.waitFor(async () => expect(await h.manager.call('root-run', 'task_status', { taskId: childTask.taskId })).toMatchObject({ status: 'completed', summary: 'Child combines nested result' }));
    root.resolve(ok('Root complete'));
    await run;
  });

  it('cancels a task subtree and never replaces a published terminal result', async () => {
    const root = deferred<CodexRunResult>();
    const gates = new Map<string, ReturnType<typeof deferred<CodexRunResult>>>();
    let childTask!: AgentTask, nested!: AgentTask;
    const cancel = vi.fn(async (id: string) => { gates.get(id)?.resolve({ ok: false, error: 'Stopped', cancelled: true }); });
    const h = await harness(async (req, context) => {
      if (!context.isChild) { childTask = await h.manager.call(req.id, 'delegate_task', { task: 'Child', target: { providerInstanceId: 'claude' } }) as AgentTask; return root.promise; }
      const gate = deferred<CodexRunResult>(); gates.set(req.id, gate);
      if (req.provider === 'claude') nested = await h.manager.call(req.id, 'delegate_task', { task: 'Grandchild', target: { providerInstanceId: 'compatible' } }) as AgentTask;
      return gate.promise;
    }, cancel);
    const run = h.start();
    await vi.waitFor(() => expect(nested?.taskId).toBeTruthy());
    await h.manager.cancelTask('root-run', childTask.taskId);
    await vi.waitFor(async () => expect(await h.manager.call('root-run', 'task_status', { taskId: childTask.taskId })).toMatchObject({ status: 'cancelled' }));
    expect(cancel).toHaveBeenCalledWith(nested.childRunId);
    const settled = await h.manager.call('root-run', 'task_status', { taskId: childTask.taskId });
    const callCount = cancel.mock.calls.length;
    await h.manager.call('root-run', 'task_cancel', { taskId: childTask.taskId });
    expect(cancel).toHaveBeenCalledTimes(callCount);
    expect(await h.manager.call('root-run', 'task_status', { taskId: childTask.taskId })).toEqual(settled);
    root.resolve(ok('Root complete')); await run;
  });

  it('stops the whole run, waits for children, and never resumes a cancelled parent', async () => {
    const root = deferred<CodexRunResult>(), child = deferred<CodexRunResult>();
    let task!: AgentTask;
    const execute = vi.fn<AgentOrchestratorOptions['execute']>(async (req, context) => {
      if (context.isChild) return child.promise;
      task = await h.manager.call(req.id, 'delegate_task', { task: 'Child' }) as AgentTask;
      return root.promise;
    });
    const h = await harness(execute, vi.fn(async id => { if (id === 'root-run') root.resolve(ok('Stopped parent')); else child.resolve(ok('Stopped child')); }));
    const run = h.start(); await vi.waitFor(() => expect(task?.taskId).toBeTruthy());
    await h.manager.cancelRun('root-run');
    await expect(run).resolves.toMatchObject({ ok: false, cancelled: true });
    expect(execute).toHaveBeenCalledTimes(2);
    expect(h.trace.filter(step => step.kind === 'task').at(-1)).toMatchObject({ task: { status: 'cancelled' } });
  });

  it('validates provider models, options, access and planning inheritance before starting work', async () => {
    const root = deferred<CodexRunResult>(); let started = false;
    const execute = vi.fn<AgentOrchestratorOptions['execute']>(async (_req, context) => { if (context.isChild) return ok('Planned'); started = true; return root.promise; });
    const h = await harness(execute); const run = h.start(request({ mode: 'editor', access: 'editor', approval: 'supervised' }));
    await vi.waitFor(() => expect(started).toBe(true));
    await expect(h.manager.call('root-run', 'delegate_task', { task: 'Invalid model', target: { providerInstanceId: 'claude', model: 'not-available' } })).rejects.toThrow('Choose a model');
    await expect(h.manager.call('root-run', 'delegate_task', { task: 'Invalid effort', target: { options: { reasoningEffort: 'ultra' } } })).rejects.toThrow('does not support');
    await expect(h.manager.call('root-run', 'delegate_task', { task: 'Escalate', access: 'computer' })).rejects.toThrow();
    const task = await h.manager.call('root-run', 'delegate_task', { task: 'Plan', target: { providerInstanceId: 'claude' }, mode: 'wait' }) as AgentTask;
    expect(task.status).toBe('completed');
    expect(execute.mock.calls[1]![0]).toMatchObject({ access: 'editor', mode: 'editor', provider: 'claude', model: 'claude-model', approval: 'supervised' });
    expect(await h.manager.call('root-run', 'orchestrator_capabilities', {})).toMatchObject({ runtimeMode: 'editor', interactionMode: 'plan', providers });
    root.resolve(ok('Done')); await run;
  });

  it('keeps task results across app restarts and retries without rerunning the provider', async () => {
    let task!: AgentTask;
    const h = await harness(async (req, context) => {
      if (context.isChild) return ok('Durable review');
      task = await h.manager.call(req.id, 'delegate_task', { task: 'Review', clientRequestId: 'durable', mode: 'wait' }) as AgentTask;
      return ok('Done');
    });
    await h.start();
    const execute = vi.fn<AgentOrchestratorOptions['execute']>(async req => {
      const restored = await restarted.call(req.id, 'delegate_task', { task: 'Review', clientRequestId: 'durable' }) as AgentTask;
      expect(restored).toMatchObject({ taskId: task.taskId, status: 'completed', summary: 'Durable review' });
      await restarted.call(req.id, 'task_status', { taskId: task.taskId });
      return ok('Result restored');
    });
    const restarted = new AgentOrchestrator({ directory: h.directory, providers: async () => providers, execute, cancel: async () => {} });
    await restarted.run(request({ id: 'second-root' }), () => {}, () => {});
    expect(execute).toHaveBeenCalledTimes(1);
  });

  it('rejects reading or cancelling tasks from another conversation', async () => {
    let task!: AgentTask;
    const h = await harness(async (req, context) => {
      if (context.isChild) return ok('Private task');
      if (req.threadId === 'other-thread') {
        await expect(h.manager.call(req.id, 'task_status', { taskId: task.taskId })).rejects.toThrow('does not belong');
        await expect(h.manager.cancelTask(req.id, task.taskId)).rejects.toThrow('does not belong');
        return ok('Other chat');
      }
      task = await h.manager.call(req.id, 'delegate_task', { task: 'Review', mode: 'wait' }) as AgentTask; return ok('Done');
    });
    await h.start(); await h.start(request({ id: 'other-root', threadId: 'other-thread' }));
  });

  it('delivers a user-cancelled child outcome to the parent instead of silently skipping it', async () => {
    const root = deferred<CodexRunResult>(), child = deferred<CodexRunResult>(); let task!: AgentTask;
    const execute = vi.fn<AgentOrchestratorOptions['execute']>(async (req, context) => {
      if (context.isChild) return child.promise;
      if (context.continuation) { expect(req.prompt).toContain('cancelled'); return ok('Review stopped by the user'); }
      task = await h.manager.call(req.id, 'delegate_task', { task: 'Review' }) as AgentTask;
      return root.promise;
    });
    const h = await harness(execute, vi.fn(async () => { child.resolve(ok('Work interrupted')); }));
    const run = h.start(); await vi.waitFor(() => expect(task?.taskId).toBeTruthy());
    await h.manager.cancelTask('root-run', task.taskId);
    root.resolve(ok('Parent turn ended'));
    await expect(run).resolves.toMatchObject({ text: 'Review stopped by the user' });
    expect(execute.mock.calls.filter(([, context]) => context.continuation)).toHaveLength(1);
  });

  it('waits for a cancelled run to drain before starting its steering replacement', async () => {
    const first = deferred<CodexRunResult>(); const running = deferred<void>();
    const h = await harness(async req => {
      if (req.id === 'root-run') { running.resolve(); return first.promise; }
      return ok('Replacement');
    });
    const old = h.start(); await running.promise;
    await h.manager.cancelRun('root-run');
    const replacement = h.start(request({ id: 'replacement-run' }));
    first.resolve(ok('Previous turn'));
    await expect(old).resolves.toMatchObject({ cancelled: true });
    await expect(replacement).resolves.toMatchObject({ text: 'Replacement' });
  });

  it('keeps the parent alive until a model-cancelled child actually exits', async () => {
    const child = deferred<CodexRunResult>(), parentTurnEnded = deferred<void>();
    let task!: AgentTask;
    const execute = vi.fn<AgentOrchestratorOptions['execute']>(async (req, context) => {
      if (context.isChild) return child.promise;
      task = await h.manager.call(req.id, 'delegate_task', { task: 'Review' }) as AgentTask;
      await h.manager.call(req.id, 'task_cancel', { taskId: task.taskId });
      parentTurnEnded.resolve();
      return ok('Review cancelled');
    });
    const h = await harness(execute);
    let settled = false;
    const run = h.start().then(result => { settled = true; return result; });
    await parentTurnEnded.promise;
    // Let the parent's returned turn settle, while child shutdown stays blocked.
    await new Promise(resolve => setImmediate(resolve));
    try { expect(settled).toBe(false); }
    finally { child.resolve(ok('Child exited')); await run; }
    await expect(run).resolves.toMatchObject({ text: 'Review cancelled' });
    expect(execute).toHaveBeenCalledTimes(2);
    expect(h.trace.filter(step => step.kind === 'task').at(-1)).toMatchObject({ task: { status: 'cancelled' } });
  });

  it('delivers a UI stop even when the parent ends during slow provider cancellation', async () => {
    const root = deferred<CodexRunResult>(), child = deferred<CodexRunResult>(), cancelled = deferred<void>();
    let task!: AgentTask;
    const h = await harness(async (req, context) => {
      if (context.isChild) return child.promise;
      if (context.continuation) { expect(req.prompt).toContain('cancelled'); return ok('Stopped outcome received'); }
      task = await h.manager.call(req.id, 'delegate_task', { task: 'Review' }) as AgentTask;
      return root.promise;
    }, vi.fn(async () => {
      root.resolve(ok('Parent turn ended'));
      child.resolve(ok('Child exited'));
      await cancelled.promise;
    }));
    const run = h.start();
    await vi.waitFor(() => expect(task?.taskId).toBeTruthy());
    const stopping = h.manager.cancelTask('root-run', task.taskId);
    try { await expect(run).resolves.toMatchObject({ text: 'Stopped outcome received' }); }
    finally { cancelled.resolve(); await stopping; }
  });

  it('marks unfinished saved work as interrupted after restart and removes it with its project', async () => {
    const root = deferred<CodexRunResult>(), child = deferred<CodexRunResult>(); let task!: AgentTask;
    const h = await harness(async (req, context) => {
      if (context.isChild) return child.promise;
      task = await h.manager.call(req.id, 'delegate_task', { task: 'Review' }) as AgentTask;
      return root.promise;
    });
    const old = h.start(); await vi.waitFor(() => expect(task?.taskId).toBeTruthy());
    const trace: CodexTraceEvent[] = [];
    const restarted = new AgentOrchestrator({ directory: h.directory, providers: async () => providers, cancel: async () => {}, execute: async req => {
      const recovered = await restarted.call(req.id, 'task_status', { taskId: task.taskId });
      expect(recovered).toMatchObject({ status: 'interrupted', workState: 'result_available', hasPendingChildRuns: false });
      return ok('Recovered');
    } });
    await restarted.run(request({ id: 'restarted-root' }), step => trace.push(step), () => {});
    expect(trace.some(step => step.kind === 'task' && step.task.status === 'interrupted')).toBe(true);
    // End the first instance before removing its files (simulated process exit).
    child.resolve(ok('Finished')); await vi.waitFor(async () => expect(await h.manager.call('root-run', 'task_status', { taskId: task.taskId })).toMatchObject({ status: 'completed' }));
    root.resolve(ok('Done')); await old;
    expect(await restarted.forgetProject('project')).toContain(`subagent-${task.childThreadId}`);
    expect(await readdir(h.directory)).toEqual([]);
  });

  it('routes questions from two child sessions even when their provider item IDs coincide', async () => {
    const root = deferred<CodexRunResult>(), child = deferred<CodexRunResult>();
    const h = await harness(async (req, context) => {
      if (context.continuation) return ok('Reviews received');
      if (context.isChild) {
        context.onTrace({ kind: 'question', itemId: 'same-provider-id', transport: 'reply', blocking: true, questions: [{ id: 'choice', header: '', question: 'Which timing?', options: [], allowOther: true, secret: false }] });
        return child.promise;
      }
      await Promise.all([1, 2].map(index => h.manager.call(req.id, 'delegate_task', { task: `Review ${index}` })));
      return root.promise;
    });
    const run = h.start();
    await vi.waitFor(() => expect(h.trace.filter(step => step.kind === 'question')).toHaveLength(2));
    const questions = h.trace.filter((step): step is Extract<CodexTraceEvent, { kind: 'question' }> => step.kind === 'question');
    expect(questions[0]!.itemId).not.toBe(questions[1]!.itemId);
    for (const question of questions) expect(h.manager.routeAnswer({ id: question.requestId!, itemId: question.itemId, answers: { choice: ['Quick'] } })).toMatchObject({ id: question.requestId, itemId: 'same-provider-id' });
    expect(() => h.manager.routeAnswer({ id: questions[1]!.requestId!, itemId: questions[0]!.itemId, answers: {} })).toThrow('another subagent');
    child.resolve(ok('Done'));
    root.resolve(ok('Done')); await expect(run).resolves.toMatchObject({ ok: true });
  });
});
