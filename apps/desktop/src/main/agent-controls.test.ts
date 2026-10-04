import { mkdtemp, rm } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { AgentOrchestrator, type AgentOrchestratorOptions } from './agent-orchestrator';
import type { AgentTask, OrchestrationProvider } from '../shared/agent-orchestration';
import type { CodexRunRequest, CodexRunResult } from '../shared/ipc';

const providers: OrchestrationProvider[] = ['chatgpt', 'claude', 'compatible'].map(provider => ({
  providerInstanceId: provider as any, driverKind: provider, displayName: provider,
  canRunChildTask: true, canRunCrossProviderChildTask: true, constraints: [],
  models: [{ id: `${provider}-model`, label: provider, options: [{ id: 'reasoningEffort', label: 'Reasoning', type: 'select', options: [{ id: 'medium', label: 'Medium' }, { id: 'high', label: 'High' }] }] }]
}));
const request = (patch: Partial<CodexRunRequest> = {}): CodexRunRequest => ({
  id: 'parent-run', threadId: 'parent-thread', projectId: 'project', projectName: 'Project', prompt: 'Original brief', projectJSON: '{}', mode: 'autonomous', provider: 'chatgpt', model: 'chatgpt-model', reasoningEffort: 'medium', access: 'project', schema: null, attachments: [], images: [], consentToken: null, ...patch
});
const ok = (text: string): CodexRunResult => ({ ok: true, text, access: 'project' });
function deferred<T>() { let resolve!: (result: T) => void; const promise = new Promise<T>(done => { resolve = done; }); return { promise, resolve }; }
const directories: string[] = [];
afterEach(async () => { await Promise.all(directories.splice(0).map(directory => rm(directory, { recursive: true, force: true }))); });
async function harness(options: Omit<AgentOrchestratorOptions, 'directory' | 'providers'>) {
  const directory = await mkdtemp(path.join(os.tmpdir(), 'pm-agent-controls-')); directories.push(directory);
  const manager = new AgentOrchestrator({ directory, providers: async () => providers, ...options });
  const trace: any[] = [], progress: string[] = [];
  return { manager, trace, progress, run: (req = request()) => manager.run(req, step => trace.push(step), text => progress.push(text)) };
}

describe('subagent direction and settings', () => {
  it('reads and answers a held subagent question without restarting its provider', async () => {
    const parent = deferred<CodexRunResult>(), child = deferred<CodexRunResult>(); let task!: AgentTask;
    const cancel = vi.fn(async () => {});
    const answer = vi.fn(() => true);
    const h = await harness({ cancel, answer, execute: async (req, context) => {
      if (context.isChild) {
        context.onTrace({ kind: 'question', itemId: 'provider-question', transport: 'reply', blocking: true, questions: [{ id: 'duration', header: 'Duration', question: 'Which duration should I use?', secret: false, allowOther: true, options: [] }] });
        return child.promise;
      }
      task = await h.manager.call(req.id, 'delegate_task', { task: 'Review timing' }) as AgentTask; return parent.promise;
    } });
    const run = h.run(); await vi.waitFor(() => expect(task?.taskId).toBeTruthy());
    await expect(h.manager.call('parent-run', 'task_status', { taskId: task.taskId })).resolves.toMatchObject({ questions: [{ blocking: true, questions: [{ question: 'Which duration should I use?' }] }] });
    await expect(h.manager.call('parent-run', 'task_send', { taskId: task.taskId, message: 'Use 3 seconds.' })).resolves.toMatchObject({ delivery: 'answered' });
    expect(answer).toHaveBeenCalledWith({ id: task.childRunId, itemId: 'provider-question', answers: { duration: ['Use 3 seconds.'] } }); expect(cancel).not.toHaveBeenCalled();
    await expect(h.manager.call('parent-run', 'task_status', { taskId: task.taskId })).resolves.toMatchObject({ questions: [] });
    child.resolve(ok('Timing checked'));
    await vi.waitFor(async () => expect(await h.manager.call('parent-run', 'task_status', { taskId: task.taskId })).toMatchObject({ status: 'completed' }));
    parent.resolve(ok('Done')); await run;
  });
  it('changes provider and reasoning in the same task, draining the old turn before resuming, with retry deduplication', async () => {
    const parent = deferred<CodexRunResult>(), firstChild = deferred<CodexRunResult>(), cancellation = deferred<void>();
    let task!: AgentTask;
    const execute = vi.fn<AgentOrchestratorOptions['execute']>(async (req, context) => {
      if (!context.isChild) { task = await h.manager.call(req.id, 'delegate_task', { task: 'Review animation' }) as AgentTask; return parent.promise; }
      if (req.provider === 'chatgpt') return firstChild.promise;
      expect(req.id).toBe(task.childRunId); expect(req.threadId).toBe(task.childThreadId);
      expect(req.prompt).toContain('Review animation'); expect(req.prompt).toContain('Focus on easing'); expect(req.reasoningEffort).toBe('high');
      return ok('Updated review');
    });
    const cancel = vi.fn(async () => { firstChild.resolve({ ok: false, error: 'Interrupted', cancelled: true }); await cancellation.promise; });
    const h = await harness({ execute, cancel }); const run = h.run();
    await vi.waitFor(() => expect(task?.taskId).toBeTruthy());
    const update = { taskId: task.taskId, message: 'Focus on easing', target: { providerInstanceId: 'claude', options: { reasoningEffort: 'high' } }, clientRequestId: 'update-1' };
    const sending = h.manager.call('parent-run', 'task_send', update);
    await vi.waitFor(() => expect(cancel).toHaveBeenCalledOnce());
    expect(execute).toHaveBeenCalledTimes(2);
    cancellation.resolve(); await expect(sending).resolves.toMatchObject({ taskId: task.taskId, model: 'claude-model', reasoningEffort: 'high', delivery: 'resumed' });
    await vi.waitFor(() => expect(h.trace.some(step => step.kind === 'task' && step.task.status === 'completed')).toBe(true));
    await expect(h.manager.call('parent-run', 'task_send', update)).resolves.toMatchObject({ delivery: 'already_sent' });
    expect(cancel).toHaveBeenCalledOnce();
    await expect(h.manager.call('parent-run', 'task_send', { ...update, message: 'Different update' })).rejects.toThrow('another update');
    await h.manager.call('parent-run', 'task_status', { taskId: task.taskId });
    parent.resolve(ok('Finished')); await run;
  });

  it('uses native live steering for direction when available and retains that direction in future context', async () => {
    const parent = deferred<CodexRunResult>(), child = deferred<CodexRunResult>(); let task!: AgentTask;
    const steer = vi.fn(async () => true), cancel = vi.fn(async () => {});
    const h = await harness({ cancel, steer, execute: async (req, context) => {
      if (context.isChild) return child.promise;
      task = await h.manager.call(req.id, 'delegate_task', { task: 'Review' }) as AgentTask; return parent.promise;
    } });
    const run = h.run(); await vi.waitFor(() => expect(task?.taskId).toBeTruthy());
    await expect(h.manager.call('parent-run', 'task_send', { taskId: task.taskId, message: 'Check contrast', clientRequestId: 'direction' })).resolves.toMatchObject({ delivery: 'live' });
    expect(steer).toHaveBeenCalledWith({ id: task.childRunId, prompt: 'Check contrast', images: [] }); expect(cancel).not.toHaveBeenCalled();
    child.resolve(ok('Contrast checked'));
    await vi.waitFor(async () => expect(await h.manager.call('parent-run', 'task_status', { taskId: task.taskId })).toMatchObject({ status: 'completed' }));
    await expect(h.manager.call('parent-run', 'task_send', { taskId: task.taskId, message: 'More work' })).rejects.toThrow('finished');
    parent.resolve(ok('Done')); await run;
  });

  it('rejects invalid effort and foreign ownership before interrupting work', async () => {
    const parent = deferred<CodexRunResult>(), child = deferred<CodexRunResult>(); let task!: AgentTask;
    const cancel = vi.fn(async () => { child.resolve(ok('Stopped')); });
    const h = await harness({ cancel, execute: async (req, context) => {
      if (context.isChild) return child.promise;
      if (req.threadId === 'other') { await expect(h.manager.call(req.id, 'task_send', { taskId: task.taskId, message: 'Foreign' })).rejects.toThrow('does not belong'); return ok('Other done'); }
      task = await h.manager.call(req.id, 'delegate_task', { task: 'Review' }) as AgentTask; return parent.promise;
    } });
    const run = h.run(); await vi.waitFor(() => expect(task?.taskId).toBeTruthy());
    await expect(h.manager.call('parent-run', 'task_send', { taskId: task.taskId, target: { options: { reasoningEffort: 'ultra' } } })).rejects.toThrow('does not support');
    await h.run(request({ id: 'other-run', threadId: 'other' })); expect(cancel).not.toHaveBeenCalled();
    await h.manager.cancelRun('parent-run'); parent.resolve(ok('Stopped')); await run;
  });

  it('does not restart a child if the parent is stopped during its reconfiguration barrier', async () => {
    const parent = deferred<CodexRunResult>(), child = deferred<CodexRunResult>(), cancellation = deferred<void>(), interrupted = deferred<void>(); let task!: AgentTask;
    const execute = vi.fn<AgentOrchestratorOptions['execute']>(async (req, context) => {
      if (context.isChild) return child.promise;
      task = await h.manager.call(req.id, 'delegate_task', { task: 'Review' }) as AgentTask; return parent.promise;
    });
    const h = await harness({ execute, cancel: async id => { if (id === 'parent-run') parent.resolve(ok('Stopped')); else { child.resolve(ok('Stopped')); interrupted.resolve(); await cancellation.promise; } } });
    const run = h.run(); await vi.waitFor(() => expect(task?.taskId).toBeTruthy());
    const send = h.manager.call('parent-run', 'task_send', { taskId: task.taskId, target: { providerInstanceId: 'claude' } });
    await interrupted.promise;
    const stop = h.manager.cancelRun('parent-run'); cancellation.resolve(); await send; await stop;
    await expect(run).resolves.toMatchObject({ cancelled: true }); expect(execute).toHaveBeenCalledTimes(2);
  });

  it('can redirect a task while it waits for nested work without duplicating or abandoning that work', async () => {
    const parent = deferred<CodexRunResult>(), nested = deferred<CodexRunResult>(), resumed = deferred<void>(); let task!: AgentTask;
    let nestedStarts = 0;
    const h = await harness({ cancel: async () => {}, execute: async (req, context) => {
      if (!context.isChild) { task = await h.manager.call(req.id, 'delegate_task', { task: 'Review' }) as AgentTask; return parent.promise; }
      if (req.prompt === 'Nested work') { nestedStarts++; return nested.promise; }
      if (req.prompt.includes('SUBAGENT RESULTS')) { expect(req.prompt).toContain('Nested result'); return ok('Revised review'); }
      if (req.prompt.includes('NEW DIRECTION')) { resumed.resolve(); return ok('New direction received'); }
      await h.manager.call(req.id, 'delegate_task', { task: 'Nested work' }); return ok('Waiting for nested work');
    } });
    const run = h.run(); await vi.waitFor(() => expect(h.trace.some(step => step.kind === 'task' && step.task.taskId === task?.taskId && step.task.status === 'waiting')).toBe(true));
    await h.manager.call('parent-run', 'task_send', { taskId: task.taskId, message: 'Check easing too', target: { providerInstanceId: 'claude' } });
    await resumed.promise; expect(nestedStarts).toBe(1);
    nested.resolve(ok('Nested result'));
    await vi.waitFor(async () => expect(await h.manager.call('parent-run', 'task_status', { taskId: task.taskId })).toMatchObject({ status: 'completed', summary: 'Revised review' }));
    parent.resolve(ok('Done')); await run;
  });
});

describe('ordinary thread monitoring', () => {
  it('starts an independent cross-provider thread, watches its run, and resumes with its result', async () => {
    const watched = deferred<CodexRunResult>(); let launched: Promise<CodexRunResult> | undefined;
    const control = vi.fn(async (_request, tool, args) => {
      if (tool === 'thread_launch') {
        launched = h.run(request({ id: 'watched-run', threadId: 'watched-thread', provider: args.provider, model: args.model, reasoningEffort: args.reasoningEffort, prompt: args.message }));
        return { threadId: 'watched-thread', runId: 'watched-run', status: 'running' };
      }
      return { thread: { threadId: 'watched-thread', runId: 'watched-run', busy: true, provider: 'claude', model: 'claude-model', access: 'project' }, messages: [] };
    });
    const execute = vi.fn<AgentOrchestratorOptions['execute']>(async (req, context) => {
      if (req.id === 'watched-run') return watched.promise;
      if (context.continuation) { expect(req.prompt).toContain('WATCHED THREAD RESULTS'); expect(req.prompt).toContain('Independent result'); return ok('Watch reviewed'); }
      const thread = await h.manager.call(req.id, 'thread_launch', { title: 'Review', message: 'Independent review', target: { providerInstanceId: 'claude', options: { reasoningEffort: 'high' } }, clientRequestId: 'launch-1' }) as any;
      await h.manager.call(req.id, 'thread_watch', { threadId: thread.threadId }); return ok('Waiting');
    });
    const h = await harness({ execute, threadControl: control, cancel: async () => {} }); const run = h.run(request({ approval: 'supervised' }));
    await vi.waitFor(() => expect(h.progress).toContain('Waiting for watched threads…'));
    expect(control.mock.calls[0]![2]).toMatchObject({ provider: 'claude', model: 'claude-model', reasoningEffort: 'high', projectId: 'project', access: 'project', approval: 'supervised' });
    watched.resolve(ok('Independent result')); await launched;
    await expect(run).resolves.toMatchObject({ text: 'Watch reviewed' });
  });

  it('stopping the watcher releases it immediately and leaves the independent thread running', async () => {
    const watched = deferred<CodexRunResult>(), ready = deferred<void>(); let watchedSettled = false;
    const cancel = vi.fn(async () => {});
    const h = await harness({ cancel, threadControl: async () => ({ thread: { runId: 'watched-run', busy: true } }), execute: async (req, context) => {
      if (req.id === 'watched-run') return watched.promise;
      await h.manager.call(req.id, 'thread_watch', { threadId: 'watched-thread' }); ready.resolve(); return ok('Watching');
    } });
    const independent = h.run(request({ id: 'watched-run', threadId: 'watched-thread' })).then(result => { watchedSettled = true; return result; });
    const parent = h.run(); await ready.promise;
    await vi.waitFor(() => expect(h.progress).toContain('Waiting for watched threads…'));
    await h.manager.cancelRun('parent-run'); await expect(parent).resolves.toMatchObject({ cancelled: true });
    expect(watchedSettled).toBe(false); expect(cancel).not.toHaveBeenCalledWith('watched-run');
    watched.resolve(ok('Independent complete')); await independent;
  });

  it('rejects cycles and cross-project run IDs', async () => {
    const a = deferred<CodexRunResult>(), b = deferred<CodexRunResult>(); let started = 0;
    const h = await harness({ cancel: async () => {}, threadControl: async (_req, _tool, args) => ({ thread: { runId: `${args.threadId}-run`, busy: true } }), execute: async req => { started++; return req.id === 'a-run' ? a.promise : b.promise; } });
    const first = h.run(request({ id: 'a-run', threadId: 'a' })), second = h.run(request({ id: 'b-run', threadId: 'b' }));
    await vi.waitFor(() => expect(started).toBe(2));
    await h.manager.call('a-run', 'thread_watch', { threadId: 'b' });
    await expect(h.manager.call('b-run', 'thread_watch', { threadId: 'a' })).rejects.toThrow('cycle');
    await expect(h.manager.call('a-run', 'thread_watch', { threadId: 'foreign', runId: 'b-run' })).rejects.toThrow('another thread');
    await h.manager.call('a-run', 'thread_unwatch', { threadId: 'b' }); a.resolve(ok('A done')); b.resolve(ok('B done')); await Promise.all([first, second]);
  });

  it('a wait timeout preserves the watch and thread_read acknowledges the selected terminal run', async () => {
    const a = deferred<CodexRunResult>(), b = deferred<CodexRunResult>(); let started = 0;
    const h = await harness({ cancel: async () => {}, threadControl: async () => ({ thread: { runId: 'b-run', busy: true } }), execute: async req => { started++; return req.id === 'b-run' ? b.promise : a.promise; } });
    const first = h.run(), second = h.run(request({ id: 'b-run', threadId: 'b' })); await vi.waitFor(() => expect(started).toBe(2));
    await expect(h.manager.call('parent-run', 'thread_watch', { threadId: 'b', mode: 'wait', timeoutMs: 1 })).resolves.toMatchObject({ waitTimedOut: true, watching: true, status: 'running' });
    b.resolve(ok('B result')); await second;
    await expect(h.manager.call('parent-run', 'thread_read', { threadId: 'b' })).resolves.toMatchObject({ run: { status: 'completed', summary: 'B result' } });
    a.resolve(ok('Already reviewed')); await expect(first).resolves.toMatchObject({ text: 'Already reviewed' });
  });

  it('binds a watch to its original run even after the target starts another turn', async () => {
    const parent = deferred<CodexRunResult>(), first = deferred<CodexRunResult>(), second = deferred<CodexRunResult>(); let started = 0;
    const h = await harness({ cancel: async () => {}, threadControl: async () => ({ thread: { runId: 'first-run', busy: true } }), execute: async (req, context) => {
      started++;
      if (req.id === 'first-run') return first.promise;
      if (req.id === 'second-run') return second.promise;
      if (context.continuation) { expect(req.prompt).toContain('First turn result'); expect(req.prompt).not.toContain('Second turn result'); return ok('First run reviewed'); }
      return parent.promise;
    } });
    const watching = h.run(), firstRun = h.run(request({ id: 'first-run', threadId: 'target' })); await vi.waitFor(() => expect(started).toBe(2));
    await h.manager.call('parent-run', 'thread_watch', { threadId: 'target' }); first.resolve(ok('First turn result')); await firstRun;
    const secondRun = h.run(request({ id: 'second-run', threadId: 'target' })); await vi.waitFor(() => expect(started).toBe(3));
    parent.resolve(ok('Receive the review')); await expect(watching).resolves.toMatchObject({ text: 'First run reviewed' });
    second.resolve(ok('Second turn result')); await secondRun;
  });

  it('removing a watch releases a waiting parent without stopping the target', async () => {
    const target = deferred<CodexRunResult>();
    const h = await harness({ cancel: async () => {}, threadControl: async () => ({ thread: { runId: 'target-run', busy: true } }), execute: async req => {
      if (req.id === 'target-run') return target.promise;
      await h.manager.call(req.id, 'thread_watch', { threadId: 'target' }); return ok('No further monitoring');
    } });
    const independent = h.run(request({ id: 'target-run', threadId: 'target' })), parent = h.run();
    await vi.waitFor(() => expect(h.progress).toContain('Waiting for watched threads…'));
    await expect(h.manager.call('parent-run', 'thread_unwatch', { threadId: 'target' })).resolves.toMatchObject({ removed: 1 });
    await expect(parent).resolves.toMatchObject({ text: 'No further monitoring' });
    target.resolve(ok('Target finished')); await independent;
  });

  it('journals an ordinary managed thread workspace for project deletion after restart', async () => {
    const h = await harness({ cancel: async () => {}, execute: async () => ok('Done') });
    await h.run(request({ threadId: 'agent-thread-created' }));
    await expect(h.manager.forgetProject('project')).resolves.toContain('thread-agent-thread-created');
  });
});
