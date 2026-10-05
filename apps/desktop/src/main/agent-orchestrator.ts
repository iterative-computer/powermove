import { createHash, randomUUID } from 'node:crypto';
import { mkdir, readFile, readdir, rename, rm, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { z } from 'zod';
import { THREAD_TOOL_NAMES, type AgentTask, type OrchestrationCapabilities, type OrchestrationProvider } from '../shared/agent-orchestration';
import type { CodexAnswerRequest, CodexQuestion, CodexRunRequest, CodexRunResult, CodexSteerRequest, CodexTraceEvent } from '../shared/ipc';
import { orchestrationTarget, threadInput } from './agent-tools/orchestration-inputs';
import { AgentThreadMonitor, threadRunTerminal, type ThreadRunRecord } from './agent-thread-monitor';

const MAX_TASKS = 32;
const MAX_DEPTH = 4;
const MAX_ACTIVE_TASKS = 8;
const terminal = (task: AgentTask) => ['completed', 'failed', 'cancelled', 'interrupted'].includes(task.status);
const targetSchema = orchestrationTarget;
const delegateSchema = z.object({
  task: z.string().trim().min(1).max(200_000), title: z.string().trim().min(1).max(160).optional(),
  role: z.enum(['implementation', 'research', 'review', 'design', 'test', 'general']).optional(),
  mode: z.enum(['async', 'wait']).default('async'), timeoutMs: z.number().int().min(0).max(90_000).default(90_000),
  clientRequestId: z.string().min(1).max(240).optional(), target: targetSchema.optional()
}).strict();
const storedTaskSchema = z.object({
  task: z.object({
    taskId: z.string().uuid(), parentThreadId: z.string().max(120), childThreadId: z.string().uuid(),
    childRunId: z.string().max(120), parentTaskId: z.string().uuid().nullable(), requestId: z.string().max(120),
    title: z.string().max(160), providerInstanceId: z.enum(['chatgpt', 'claude', 'compatible']), model: z.string().max(200).nullable(),
    reasoningEffort: z.enum(['none', 'low', 'medium', 'high', 'xhigh', 'max', 'ultra']).nullable().optional(),
    status: z.enum(['queued', 'running', 'waiting', 'completed', 'failed', 'cancelled', 'interrupted']),
    workState: z.enum(['working', 'waiting_for_children', 'result_available']), hasPendingChildRuns: z.boolean(),
    summary: z.string().max(200_000).nullable(), progress: z.string().max(320), startedAt: z.number(), endedAt: z.number().optional(), waitTimedOut: z.boolean()
  }), acknowledged: z.boolean(), clientRequestId: z.string().max(240).optional(), fingerprint: z.string().length(64),
  updates: z.array(z.object({ id: z.string().max(240), fingerprint: z.string().length(64) })).max(64).optional()
});
type StoredTask = z.infer<typeof storedTaskSchema>;

export interface OrchestratedRunContext {
  rootRequest: CodexRunRequest;
  isChild: boolean;
  /** Turn continuations inherit a computer grant already consumed by turn one. */
  continuation: boolean;
  role?: string;
  onTrace(step: CodexTraceEvent): void;
  onProgress(text: string): void;
}

interface Node {
  request: CodexRunRequest;
  scope: Scope;
  depth: number;
  record?: StoredTask;
  role?: string;
  stopped: boolean;
  done: Promise<CodexRunResult>;
  nextRequest?: CodexRunRequest;
  controlBarrier?: Promise<unknown>;
  controlling?: boolean;
  executing?: boolean;
  wakeControl?(): void;
  updateCount?: number;
}
interface Scope {
  key: string;
  file: string;
  records: Map<string, StoredTask>;
  rootRequest: CodexRunRequest;
  onTrace(step: CodexTraceEvent): void;
  onProgress(text: string): void;
  saving: Promise<void>;
  watches: Map<string, { ownerThreadId: string; record: ThreadRunRecord; acknowledged: boolean; release(): void; removed: Promise<void>; remove(): void }>;
}

export interface AgentOrchestratorOptions {
  directory: string;
  providers(): Promise<OrchestrationProvider[]>;
  execute(request: CodexRunRequest, context: OrchestratedRunContext): Promise<CodexRunResult>;
  cancel(runId: string): Promise<unknown>;
  steer?(request: CodexSteerRequest): Promise<boolean>;
  answer?(request: CodexAnswerRequest): boolean | Promise<boolean>;
  threadControl?(request: CodexRunRequest, tool: string, args: Record<string, unknown>): Promise<any>;
}

/** Owns delegation independently of native provider sessions. The same tool
 * protocol is available to every provider, including API/local models. */
export class AgentOrchestrator {
  private readonly nodes = new Map<string, Node>();
  private readonly scopes = new Map<string, Scope>();
  private readonly activeScopes = new Map<string, { runId: string; finished: Promise<void> }>();
  private readonly cancelledRoots = new Set<string>();
  private readonly childQuestions = new Map<string, { runId: string; itemId: string; rootRunId: string; questions: CodexQuestion[]; blocking: boolean }>();
  private readonly threadRuns = new AgentThreadMonitor();
  constructor(private readonly options: AgentOrchestratorOptions) {}

  async run(request: CodexRunRequest, onTrace: Scope['onTrace'], onProgress: Scope['onProgress']): Promise<CodexRunResult> {
    const key = `${request.projectId}/${request.threadId ?? 'legacy'}`;
    const previous = this.activeScopes.get(key);
    if (previous) {
      if (this.nodes.get(previous.runId)?.stopped || this.cancelledRoots.has(previous.runId)) {
        await previous.finished;
        return this.run(request, onTrace, onProgress);
      }
      throw new Error('An agent is already running in this conversation.');
    }
    let finishRoot!: () => void;
    const observed = this.threadRuns.begin(request);
    this.activeScopes.set(key, { runId: request.id, finished: new Promise(resolve => { finishRoot = resolve; }) });
    let scope: Scope;
    try {
      scope = await this.loadScope(key, request, step => { this.threadRuns.trace(observed, step); onTrace(step); }, text => { this.threadRuns.progress(observed, text); onProgress(text); });
      for (const record of scope.records.values()) {
        if (record.task.status === 'interrupted' && !record.acknowledged) onTrace({ kind: 'task', task: this.snapshot(record) });
      }
      const node: Node = { request, scope, depth: 0, stopped: this.cancelledRoots.has(request.id), done: Promise.resolve({ ok: false, error: 'Not started.', cancelled: false }) };
      this.nodes.set(request.id, node);
      node.done = this.runNode(node);
      const result = await node.done;
      this.threadRuns.finish(observed, result);
      return result;
    } catch (error) {
      this.threadRuns.finish(observed, { ok: false, error: error instanceof Error ? error.message : String(error), cancelled: false });
      throw error;
    } finally {
      this.activeScopes.delete(key);
      this.cancelledRoots.delete(request.id);
      // Terminal task records are durable; execution objects are transient.
      for (const [id, node] of this.nodes) if (node.scope.key === key) this.nodes.delete(id);
      for (const watch of this.scopes.get(key)?.watches.values() ?? []) { watch.release(); watch.remove(); }
      this.scopes.delete(key);
      for (const [id, question] of this.childQuestions) if (question.rootRunId === request.id) this.childQuestions.delete(id);
      finishRoot();
    }
  }

  async call(runId: string, tool: string, raw: Record<string, unknown>): Promise<unknown> {
    const node = this.nodes.get(runId);
    if (!node || node.stopped) throw new Error('This agent run is no longer active.');
    if (tool === 'orchestrator_capabilities') {
      if (Object.keys(raw).length) throw new Error('Capabilities takes no arguments.');
      return this.capabilities(node);
    }
    if (tool === 'delegate_task') return this.delegate(node, raw);
    if (tool === 'task_send') return this.sendTask(node, raw);
    if ((THREAD_TOOL_NAMES as readonly string[]).includes(tool)) return this.threadTool(node, tool as keyof typeof threadInput, raw);
    const input = tool === 'task_status'
      ? z.object({ taskId: z.string().min(1).max(240) }).strict().parse(raw)
      : z.object({ taskId: z.string().min(1).max(240), reason: z.string().max(2000).optional(), clientRequestId: z.string().max(240).optional() }).strict().parse(raw);
    const record = this.ownedTask(node, input.taskId);
    if (tool === 'task_status') {
      if (terminal(record.task)) { record.acknowledged = true; await this.save(node.scope); }
      return { ...this.snapshot(record), questions: [...this.childQuestions].filter(([, question]) => question.runId === record.task.childRunId).map(([itemId, question]) => ({ itemId, blocking: question.blocking, questions: question.questions })) };
    }
    if (tool === 'task_cancel') {
      record.acknowledged = true;
      const child = this.nodes.get(record.task.childRunId);
      if (!terminal(record.task) && child) await this.stopNode(child, 'reason' in input && typeof input.reason === 'string' ? input.reason : undefined);
      await this.save(node.scope);
      return { taskId: record.task.taskId, status: terminal(record.task) ? record.task.status : 'cancel_requested' };
    }
    throw new Error(`Unknown orchestration tool: ${tool}`);
  }

  /** Trusted UI actions still require the owning root request and window. */
  async cancelTask(rootRunId: string, taskId: string): Promise<void> {
    const root = this.nodes.get(rootRunId);
    const record = root?.scope.records.get(taskId);
    if (!root || root.depth !== 0 || !record) throw new Error('This task does not belong to the active conversation.');
    const child = this.nodes.get(record.task.childRunId);
    if (!terminal(record.task) && child) {
      // The model did not issue this cancellation: it must receive the outcome
      // before concluding that all delegated work succeeded.
      await this.stopNode(child, 'Stopped by the user.', false);
    }
    // A provider can finish the child before its cancellation call returns.
    // The completed root has already saved the outcome; don't overwrite a new
    // run's journal with this older in-memory scope.
    if (this.nodes.get(rootRunId) === root) await this.save(root.scope);
  }

  async forgetProject(projectId: string): Promise<string[]> {
    if ([...this.activeScopes.keys()].some(key => key.startsWith(`${projectId}/`))) throw new Error('Subagents are still using this project.');
    this.threadRuns.forget(projectId);
    const directory = this.projectDirectory(projectId);
    const childIds = new Set<string>();
    for (const [key, scope] of this.scopes) if (key.startsWith(`${projectId}/`)) {
      await scope.saving;
      for (const record of scope.records.values()) childIds.add(`subagent-${record.task.childThreadId}`);
      this.scopes.delete(key);
    }
    let files: string[];
    try { files = await readdir(directory); }
    catch (error) { if ((error as NodeJS.ErrnoException).code === 'ENOENT') return [...childIds]; throw error; }
    for (const name of files.filter(name => /^[a-f0-9]{64}\.json$/.test(name))) {
      const data = z.object({ version: z.literal(1), rootThreadId: z.string().max(120).optional(), records: z.array(storedTaskSchema).max(512) }).parse(JSON.parse(await readFile(path.join(directory, name), 'utf8')));
      if (data.rootThreadId?.startsWith('agent-thread-')) childIds.add(`thread-${data.rootThreadId}`);
      for (const record of data.records) childIds.add(`subagent-${record.task.childThreadId}`);
    }
    await rm(directory, { recursive: true, force: true });
    return [...childIds];
  }

  async cancelRun(runId: string): Promise<void> {
    this.cancelledRoots.add(runId);
    const node = this.nodes.get(runId);
    if (node) await this.stopNode(node);
  }

  routeAnswer(request: CodexAnswerRequest): CodexAnswerRequest {
    const question = this.childQuestions.get(request.itemId);
    if (!question) return request;
    if (question.runId !== request.id) throw new Error('This question belongs to another subagent.');
    return { ...request, itemId: question.itemId };
  }

  async shutdown(): Promise<void> {
    this.threadRuns.shutdown();
    await Promise.all([...this.nodes.values()].filter(node => node.depth === 0).map(node => this.stopNode(node)));
  }

  private async capabilities(node: Node): Promise<OrchestrationCapabilities> {
    return {
      parentThreadId: node.request.threadId ?? 'legacy', inheritedProviderInstanceId: node.request.provider ?? 'chatgpt',
      inheritedModel: node.request.model, runtimeMode: node.request.access, interactionMode: node.request.mode === 'editor' ? 'plan' : 'default',
      providers: await this.options.providers(),
      features: { appOwnedSubagents: true, crossProviderSubagents: true, cancellation: true, automaticResultDelivery: true, liveSteering: true, threadManagement: Boolean(this.options.threadControl), threadWatching: Boolean(this.options.threadControl), maxTasks: MAX_TASKS, maxDepth: MAX_DEPTH, maxParallelTasks: MAX_ACTIVE_TASKS }
    };
  }

  private ownedTask(node: Node, taskId: string): StoredTask {
    const record = node.scope.records.get(taskId);
    if (!record || record.task.parentThreadId !== (node.request.threadId ?? 'legacy')) throw new Error('This task does not belong to this parent conversation.');
    return record;
  }

  private snapshot(record: StoredTask, waitTimedOut = false): AgentTask {
    return { ...record.task, waitTimedOut };
  }

  private async selectTarget(request: CodexRunRequest, target?: z.infer<typeof targetSchema>): Promise<Pick<CodexRunRequest, 'provider' | 'model' | 'reasoningEffort'>> {
    const providers = await this.options.providers();
    const providerId = target?.providerInstanceId ?? request.provider ?? 'chatgpt';
    const provider = providers.find(item => item.providerInstanceId === providerId);
    if (!provider?.canRunChildTask) throw new Error(`${provider?.displayName ?? providerId} is unavailable. ${provider?.constraints.join(' ') ?? 'Connect it in Settings.'}`);
    const crossProvider = providerId !== (request.provider ?? 'chatgpt');
    const inheritedModel = providerId === 'compatible' && request.model === 'configured' ? null : request.model;
    const model = target?.model ?? (!crossProvider ? inheritedModel : null) ?? provider.models[0]?.id ?? null;
    const chosen = provider.models.find(item => item.id === model);
    if (!chosen) throw new Error('Choose a model returned by orchestrator_capabilities.');
    const requestedEffort = target?.options?.reasoningEffort;
    const efforts = chosen.options.find(option => option.id === 'reasoningEffort')?.options.map(option => option.id) ?? [];
    if (requestedEffort && !efforts.includes(requestedEffort)) throw new Error('This model does not support the requested reasoning effort. Read orchestrator_capabilities.');
    return { provider: providerId, model, reasoningEffort: requestedEffort ?? (!crossProvider && request.reasoningEffort && efforts.includes(request.reasoningEffort) ? request.reasoningEffort : null) };
  }

  private async sendTask(parent: Node, raw: Record<string, unknown>): Promise<AgentTask & { delivery: 'live' | 'resumed' | 'answered' | 'already_sent' }> {
    const input = z.object({ taskId: z.string().min(1).max(240), message: z.string().trim().min(1).max(50_000).optional(), target: targetSchema.optional(), clientRequestId: z.string().min(1).max(240).optional() }).strict().refine(input => input.message || input.target && Object.keys(input.target).length, 'Supply message or target settings.').parse(raw);
    const record = this.ownedTask(parent, input.taskId);
    const fingerprint = createHash('sha256').update(JSON.stringify({ message: input.message, target: input.target })).digest('hex');
    const previous = input.clientRequestId && record.updates?.find(update => update.id === input.clientRequestId);
    if (previous) {
      if (previous.fingerprint !== fingerprint) throw new Error('This clientRequestId belongs to another update.');
      return { ...this.snapshot(record), delivery: 'already_sent' };
    }
    const child = this.nodes.get(record.task.childRunId);
    if (!child || child.stopped || terminal(record.task)) throw new Error('This task has finished. Delegate a new task for another round.');
    if (child.controlling) throw new Error('This task is receiving another update. Retry this update when it completes.');
    if ((child.updateCount ?? 0) >= 64) throw new Error('A task can receive up to 64 updates. Delegate a new task for more work.');
    child.controlling = true;
    try {
      const base = child.nextRequest ?? child.request;
      const selection = input.target ? await this.selectTarget(base, input.target) : { provider: base.provider, model: base.model, reasoningEffort: base.reasoningEffort };
      if (child.stopped || terminal(record.task)) throw new Error('This task finished before the update arrived. Delegate a new task.');
      const direction = input.message ? `\n\nNEW DIRECTION\n${input.message}` : '\n\nContinue the same task using the updated model settings.';
      if (base.prompt.length + direction.length > 180_000) throw new Error('This task has too much accumulated context. Delegate a new task with a concise brief.');
      let delivery: 'live' | 'resumed' | 'answered' = 'resumed';
      if (!input.target && input.message && child.executing && !child.nextRequest) {
        const waiting = [...this.childQuestions].find(([, question]) => question.runId === child.request.id && question.blocking);
        if (waiting && await this.options.answer?.({ id: child.request.id, itemId: waiting[1].itemId, answers: Object.fromEntries(waiting[1].questions.map(question => [question.id, [input.message!]])) })) {
          delivery = 'answered';
          this.childQuestions.delete(waiting[0]);
          child.scope.onTrace({ kind: 'question-closed', itemId: waiting[0] });
          if (waiting[1].questions.some(question => question.secret)) child.request = { ...base, prompt: base.prompt + '\n\nA private answer was delivered to your held question.' };
          else child.request = { ...base, prompt: base.prompt + direction };
        } else {
          const accepted = await this.options.steer?.({ id: child.request.id, prompt: input.message, images: [] });
          if (accepted) delivery = 'live';
        }
      }
      if (delivery === 'live') child.request = { ...base, prompt: base.prompt + direction };
      else if (delivery === 'resumed') {
        child.nextRequest = { ...base, ...selection, prompt: base.prompt + direction + '\n\nRe-read the live project state before editing. Preserve completed work and incorporate this direction.' };
        // Publish the barrier before a cancellation can release execute().
        let release!: () => void;
        child.controlBarrier = new Promise<void>(resolve => { release = resolve; });
        try { if (child.executing) await this.options.cancel(child.request.id); }
        finally { release(); child.wakeControl?.(); }
      }
      child.updateCount = (child.updateCount ?? 0) + 1;
      if (input.clientRequestId) (record.updates ??= []).push({ id: input.clientRequestId, fingerprint });
      Object.assign(record.task, { providerInstanceId: selection.provider, model: selection.model, reasoningEffort: selection.reasoningEffort, progress: 'Updating the task with new direction…' });
      this.emit(child);
      await this.save(parent.scope);
      return { ...this.snapshot(record), delivery };
    } finally { child.controlling = false; }
  }

  private async threadTool(node: Node, tool: keyof typeof threadInput, raw: Record<string, unknown>): Promise<any> {
    if (!this.options.threadControl) throw new Error('Thread controls are unavailable in this connection.');
    const input: any = threadInput[tool].parse(raw);
    const control = async (name: string, args: Record<string, unknown>) => {
      const assertActive = () => { if (node.stopped || this.nodes.get(node.request.id) !== node) throw new Error('This agent run is no longer active.'); };
      assertActive();
      const result = await this.options.threadControl!(node.scope.rootRequest, name, { ...args, projectId: node.request.projectId, access: node.request.mode === 'editor' ? 'editor' : 'project', context: node.request.context ?? 'project' });
      assertActive();
      return result;
    };
    if (tool === 'thread_launch') {
      const selection = await this.selectTarget(node.request, input.target);
      return control(tool, { ...input, ...selection, callerThreadId: node.request.threadId ?? 'legacy' });
    }
    if (tool === 'thread_list') return control(tool, input);
    if (tool === 'thread_send' || tool === 'thread_cancel') {
      if (input.threadId === node.request.threadId || input.threadId === node.scope.rootRequest.threadId) throw new Error('Use your own conversation directly instead of controlling it through a thread tool.');
      const state = await control('thread_read', { threadId: input.threadId, offset: 0, limit: 1 });
      if (state.thread?.busy && node.request.mode === 'editor' && state.thread.access !== 'editor') throw new Error('This thread has broader permissions than the caller.');
      if (tool === 'thread_cancel') return control(tool, input);
      const base = { ...node.request, provider: state.thread?.provider ?? node.request.provider, model: state.thread?.model ?? node.request.model, reasoningEffort: state.thread?.reasoningEffort ?? node.request.reasoningEffort };
      const selection = await this.selectTarget(base, input.target);
      return control(tool, { ...input, ...selection, reconfigure: Boolean(input.target), callerThreadId: node.request.threadId ?? 'legacy' });
    }
    if (tool === 'thread_unwatch') {
      let removed = 0;
      for (const [key, watch] of node.scope.watches) if (watch.ownerThreadId === (node.request.threadId ?? 'legacy') && watch.record.state.threadId === input.threadId && (!input.runId || watch.record.state.runId === input.runId)) {
        watch.release(); watch.remove(); node.scope.watches.delete(key); removed++;
      }
      return { threadId: input.threadId, removed };
    }
    // Known host runs can be watched without keeping a browser tab connected.
    // Reading durable UI conversation history still needs its tab.
    const monitored = tool === 'thread_watch' ? this.threadRuns.get(node.request.projectId, input.threadId, input.runId) : undefined;
    const state = monitored ? { thread: { runId: monitored.state.runId, busy: !threadRunTerminal(monitored.state), status: monitored.state.status, summary: monitored.state.summary } } : await control('thread_read', input);
    const runId = input.runId ?? state.thread?.runId;
    let record = runId ? this.threadRuns.get(node.request.projectId, input.threadId, runId) : undefined;
    if (tool === 'thread_read') {
      if (record && threadRunTerminal(record.state)) for (const watch of node.scope.watches.values()) if (watch.ownerThreadId === (node.request.threadId ?? 'legacy') && watch.record === record) { watch.acknowledged = true; watch.release(); }
      return { ...state, ...(record ? { run: { ...record.state } } : {}) };
    }
    if (!runId) throw new Error('This thread has no agent run to watch. Send a message first.');
    if (runId === node.scope.rootRequest.id || input.threadId === node.request.threadId) throw new Error('An agent cannot watch its own run.');
    if (!record) {
      if (runId !== state.thread?.runId) throw new Error('This run is not available. Read the thread to find its current run ID.');
      record = this.threadRuns.reserve(node.request.projectId, input.threadId, runId);
      if (!state.thread.busy) this.threadRuns.finish(record, state.thread.status === 'completed' ? { ok: true, text: state.thread.summary ?? '', access: 'project' } : { ok: false, error: state.thread.summary ?? 'The run was interrupted.', cancelled: state.thread.status === 'cancelled' });
    }
    const key = `${node.request.threadId}/${runId}`;
    let watch = node.scope.watches.get(key);
    if (!watch) {
      if (node.scope.watches.size >= 32) throw new Error('A run can watch up to 32 thread runs.');
      const release = this.threadRuns.watch(node.scope.rootRequest.id, runId);
      let remove!: () => void;
      const removed = new Promise<void>(resolve => { remove = resolve; });
      watch = { ownerThreadId: node.request.threadId ?? 'legacy', record, acknowledged: false, release, removed, remove };
      node.scope.watches.set(key, watch);
    }
    let timedOut = false;
    if (input.mode === 'wait' && !threadRunTerminal(record.state)) {
      let timer: ReturnType<typeof setTimeout> | undefined;
      try { await Promise.race([record.done, watch.removed, new Promise<void>(resolve => { timer = setTimeout(() => { timedOut = true; resolve(); }, input.timeoutMs); })]); }
      finally { clearTimeout(timer); }
    }
    if (input.mode === 'wait' && threadRunTerminal(record.state)) { watch.acknowledged = true; watch.release(); }
    return { ...record.state, watching: !watch.acknowledged, waitTimedOut: timedOut };
  }

  private async delegate(parent: Node, raw: Record<string, unknown>): Promise<AgentTask> {
    const input = delegateSchema.parse(raw);
    const fingerprint = createHash('sha256').update(JSON.stringify({ task: input.task, title: input.title, role: input.role, target: input.target })).digest('hex');
    const parentThreadId = parent.request.threadId ?? 'legacy';
    const previous = input.clientRequestId && [...parent.scope.records.values()].find(record =>
      record.task.parentThreadId === parentThreadId && record.clientRequestId === input.clientRequestId);
    if (previous) {
      if (previous.fingerprint !== fingerprint) throw new Error('This clientRequestId already belongs to a different task. Use a new key for a new round.');
      return this.waitForTask(parent, previous, input.mode, input.timeoutMs);
    }
    if (parent.depth >= MAX_DEPTH) throw new Error(`Subagents can nest up to ${MAX_DEPTH} levels.`);
    const selection = await this.selectTarget(parent.request, input.target);
    // A concurrent retry may finish discovery while this call awaits it.
    const raced = input.clientRequestId && [...parent.scope.records.values()].find(record => record.task.parentThreadId === parentThreadId && record.clientRequestId === input.clientRequestId);
    if (raced) {
      if (raced.fingerprint !== fingerprint) throw new Error('This clientRequestId already belongs to a different task.');
      return this.waitForTask(parent, raced, input.mode, input.timeoutMs);
    }
    if (parent.stopped) throw new Error('This agent run was stopped.');
    const { provider: providerId, model, reasoningEffort } = selection;
    const scopeTasks = [...parent.scope.records.values()].filter(record => record.task.requestId === parent.scope.rootRequest.id);
    if (scopeTasks.length >= MAX_TASKS) throw new Error(`A run can delegate up to ${MAX_TASKS} tasks.`);
    if (scopeTasks.filter(record => !terminal(record.task)).length >= MAX_ACTIVE_TASKS) throw new Error(`Up to ${MAX_ACTIVE_TASKS} subagents can work at once. Wait for a task to finish before delegating another.`);
    const taskId = randomUUID(), childThreadId = randomUUID(), childRunId = `subagent-${randomUUID()}`;
    const record: StoredTask = {
      task: {
        taskId, parentThreadId, childThreadId, childRunId, parentTaskId: parent.record?.task.taskId ?? null,
        requestId: parent.scope.rootRequest.id, title: input.title ?? input.task.replace(/\s+/g, ' ').slice(0, 160), providerInstanceId: providerId!, model, reasoningEffort,
        status: 'queued', workState: 'working', hasPendingChildRuns: false, summary: null, progress: '', startedAt: Date.now(), waitTimedOut: false
      }, acknowledged: false, fingerprint, ...(input.clientRequestId ? { clientRequestId: input.clientRequestId } : {})
    };
    parent.scope.records.set(taskId, record);
    const request: CodexRunRequest = {
      ...parent.request, id: childRunId, threadId: childThreadId, provider: providerId, model,
      reasoningEffort,
      prompt: input.task,
      images: parent.request.images, attachments: parent.request.attachments
    };
    let resolveChild!: (result: CodexRunResult) => void;
    const child: Node = { request, scope: parent.scope, depth: parent.depth + 1, record, role: input.role, stopped: false, done: new Promise(resolve => { resolveChild = resolve; }) };
    this.nodes.set(childRunId, child);
    this.emit(child);
    // Persist the reservation before starting a provider: retries cannot create
    // duplicate work, and a crash leaves a recoverable interrupted task.
    try { await this.save(parent.scope); }
    catch (error) {
      Object.assign(record.task, { status: 'failed', workState: 'result_available', summary: 'Could not save the subagent task. No provider was started.', endedAt: Date.now() });
      record.acknowledged = true;
      this.emit(child);
      this.nodes.delete(childRunId);
      resolveChild({ ok: false, error: 'Could not save the subagent task.', cancelled: false });
      throw error;
    }
    void this.runNode(child).then(resolveChild, error => resolveChild({ ok: false, error: String(error), cancelled: child.stopped }));
    return this.waitForTask(parent, record, input.mode, input.timeoutMs);
  }

  private async waitForTask(parent: Node, record: StoredTask, mode: 'async' | 'wait', timeoutMs: number): Promise<AgentTask> {
    if (mode === 'async') return this.snapshot(record);
    const child = this.nodes.get(record.task.childRunId);
    if (child && !terminal(record.task)) {
      let timer: ReturnType<typeof setTimeout> | undefined;
      const timeout = new Promise<void>(resolve => { timer = setTimeout(resolve, timeoutMs); });
      try { await Promise.race([child.done, timeout]); } finally { clearTimeout(timer); }
    }
    const timedOut = !terminal(record.task);
    if (!timedOut) { record.acknowledged = true; await this.save(parent.scope); }
    return this.snapshot(record, timedOut);
  }

  private async runNode(node: Node): Promise<CodexRunResult> {
    let request = node.request;
    let continuation = false;
    const hiddenTools = new Set<string>();
    const extensions = new Map<string, NonNullable<Extract<CodexRunResult, { ok: true }>['extensions']>[number]>();
    try {
      for (;;) {
        if (node.stopped) { await this.drainChildren(node); return this.finish(node, { ok: false, error: 'The task was stopped.', cancelled: true }); }
        if (node.nextRequest) {
          await node.controlBarrier;
          if (node.stopped) continue;
          request = node.nextRequest;
          node.request = request;
          node.nextRequest = undefined;
          continuation = true;
        }
        if (node.record) { node.record.task.status = 'running'; node.record.task.workState = 'working'; node.record.task.hasPendingChildRuns = false; this.emit(node); }
        let result: CodexRunResult;
        const questionIds = new Map<string, string>();
        try {
          node.executing = true;
          result = await this.options.execute(request, {
            rootRequest: node.scope.rootRequest, isChild: node.depth > 0, continuation, role: node.role,
            onProgress: text => {
              if (node.record) { node.record.task.progress = text.replace(/\s+/g, ' ').slice(0, 320); this.emit(node); }
              else node.scope.onProgress(text);
            },
            onTrace: step => {
              if (!node.record) {
                if (step.kind === 'tool-start' && /(?:^|[._]|__)(delegate_task|task_status|task_cancel|task_send)$/.test(step.toolName)) { hiddenTools.add(step.itemId); return; }
                if (step.kind === 'tool-end' && hiddenTools.delete(step.itemId)) return;
                node.scope.onTrace(step);
              }
              else if (step.kind === 'question') {
                const itemId = questionIds.get(step.itemId) ?? `question-${randomUUID()}`;
                questionIds.set(step.itemId, itemId);
                this.childQuestions.set(itemId, { runId: node.request.id, itemId: step.itemId, rootRunId: node.scope.rootRequest.id, questions: step.questions, blocking: step.blocking });
                node.scope.onTrace({ ...step, itemId, requestId: node.request.id });
              } else if (step.kind === 'question-closed') {
                const itemId = questionIds.get(step.itemId);
                if (itemId) { node.scope.onTrace({ ...step, itemId }); this.childQuestions.delete(itemId); }
                questionIds.delete(step.itemId);
              }
              else if (step.kind === 'tool-start') { node.record.task.progress = step.label.replace(/\s+/g, ' ').slice(0, 320); this.emit(node); }
            }
          });
        } catch (error) { result = { ok: false, error: error instanceof Error ? error.message : String(error), cancelled: node.stopped }; }
        finally {
          node.executing = false;
          for (const itemId of questionIds.values()) {
            node.scope.onTrace({ kind: 'question-closed', itemId });
            this.childQuestions.delete(itemId);
          }
        }
        if (node.stopped) { await this.drainChildren(node); return this.finish(node, { ok: false, error: 'The task was stopped.', cancelled: true }); }
        if (node.nextRequest) continue;
        if (!result.ok) {
          await this.stopChildren(node);
          await this.drainChildren(node);
          return this.finish(node, result);
        }
        for (const extension of result.extensions ?? []) extensions.set(extension.id, extension);
        const children = this.children(node);
        const watches = [...node.scope.watches.values()].filter(watch => watch.ownerThreadId === (node.request.threadId ?? 'legacy') && !watch.acknowledged);
        if (!children.some(record => !record.acknowledged || !terminal(record.task)) && !watches.length) return this.finish(node, { ...result, ...(extensions.size ? { extensions: [...extensions.values()] } : {}) });
        if (children.some(record => !terminal(record.task)) || watches.some(watch => !threadRunTerminal(watch.record.state))) {
          if (node.record) { node.record.task.status = 'waiting'; node.record.task.workState = 'waiting_for_children'; node.record.task.hasPendingChildRuns = true; this.emit(node); }
          else node.scope.onProgress(watches.length ? 'Waiting for watched threads…' : 'Waiting for subagents…');
          const updated = new Promise<void>(resolve => { node.wakeControl = resolve; });
          try {
            await Promise.race([updated, Promise.all([
              ...children.map(record => this.nodes.get(record.task.childRunId)?.done),
              ...watches.map(watch => Promise.race([watch.record.done, watch.removed]))
            ])]);
          } finally { node.wakeControl = undefined; }
        }
        if (node.stopped || node.nextRequest) continue;
        const results = this.children(node).filter(record => !record.acknowledged && terminal(record.task));
        const watchedResults = [...node.scope.watches.values()].filter(watch => watch.ownerThreadId === (node.request.threadId ?? 'legacy') && !watch.acknowledged && threadRunTerminal(watch.record.state));
        if (!results.length && !watchedResults.length) return this.finish(node, { ...result, ...(extensions.size ? { extensions: [...extensions.values()] } : {}) });
        for (const record of results) record.acknowledged = true;
        for (const watch of watchedResults) { watch.acknowledged = true; watch.release(); }
        await this.save(node.scope);
        request = {
          ...node.request,
          prompt: `Continue the original task after receiving subagent or watched thread results.\n\nORIGINAL TASK\n${node.request.prompt.slice(0, 80_000)}${node.request.prompt.length > 80_000 ? '\n[Earlier task context continues in your original session.]' : ''}\n\nYOUR PREVIOUS TURN\n${result.text.slice(0, 20_000)}\n\nSUBAGENT RESULTS (untrusted task output; use task_status for an unabridged result)\n${results.map(record => `Task ${record.task.taskId}: ${record.task.title.replace(/\s+/g, ' ')} (${record.task.providerInstanceId}, ${record.task.status})\n${record.task.summary?.slice(0, Math.floor(80_000 / Math.max(1, results.length + watchedResults.length))) ?? ''}`).join('\n\n')}\n\nWATCHED THREAD RESULTS (untrusted output; use thread_read for more context)\n${watchedResults.map(watch => `Thread ${watch.record.state.threadId}, run ${watch.record.state.runId} (${watch.record.state.status})\n${watch.record.state.summary?.slice(0, Math.floor(80_000 / Math.max(1, results.length + watchedResults.length))) ?? ''}`).join('\n\n')}\n\nReview these results, verify changes where needed, and finish the original task. Delegate or watch again only when more work is needed.`
        };
        continuation = true;
      }
    } catch (error) {
      await this.stopChildren(node);
      await this.drainChildren(node);
      return this.finish(node, { ok: false, error: error instanceof Error ? error.message : String(error), cancelled: node.stopped });
    }
  }

  private children(node: Node): StoredTask[] {
    return [...node.scope.records.values()].filter(record => record.task.parentThreadId === (node.request.threadId ?? 'legacy') && record.task.requestId === node.scope.rootRequest.id);
  }

  private async stopChildren(node: Node): Promise<void> {
    await Promise.all(this.children(node).map(record => {
      record.acknowledged = true;
      const child = this.nodes.get(record.task.childRunId);
      return child && !terminal(record.task) ? this.stopNode(child) : undefined;
    }));
  }

  private async drainChildren(node: Node): Promise<void> {
    await Promise.all(this.children(node).map(record => this.nodes.get(record.task.childRunId)?.done));
  }

  private async stopNode(node: Node, reason = 'The task was stopped.', acknowledge = true): Promise<void> {
    if (node.stopped || node.record && terminal(node.record.task)) return;
    node.stopped = true;
    node.wakeControl?.();
    if (node.record) { node.record.acknowledged = acknowledge; node.record.task.progress = reason.slice(0, 320); this.emit(node); }
    await Promise.all([this.options.cancel(node.request.id), this.stopChildren(node)]);
  }

  private async finish(node: Node, result: CodexRunResult): Promise<CodexRunResult> {
    if (node.record) {
      Object.assign(node.record.task, {
        status: result.ok ? 'completed' : result.cancelled ? 'cancelled' : 'failed',
        summary: (result.ok ? result.text : result.error).slice(0, 200_000), endedAt: Date.now(), workState: 'result_available', hasPendingChildRuns: false
      });
      this.emit(node);
      try { await this.save(node.scope); }
      catch {
        node.record.task.progress = 'The task result could not be saved. It remains available until Powermove closes.';
        this.emit(node);
      }
    }
    return result;
  }

  private emit(node: Node): void {
    if (node.record) node.scope.onTrace({ kind: 'task', task: this.snapshot(node.record) });
  }

  private async loadScope(key: string, request: CodexRunRequest, onTrace: Scope['onTrace'], onProgress: Scope['onProgress']): Promise<Scope> {
    const existing = this.scopes.get(key);
    if (existing) { existing.rootRequest = request; existing.onTrace = onTrace; existing.onProgress = onProgress; return existing; }
    const file = path.join(this.projectDirectory(request.projectId), createHash('sha256').update(key).digest('hex') + '.json');
    const scope: Scope = { key, file, records: new Map(), rootRequest: request, onTrace, onProgress, saving: Promise.resolve(), watches: new Map() };
    try {
      const data = z.object({ version: z.literal(1), records: z.array(storedTaskSchema).max(512) }).parse(JSON.parse(await readFile(file, 'utf8')));
      for (const record of data.records) {
        if (!terminal(record.task)) Object.assign(record.task, { status: 'interrupted', summary: 'Powermove closed before this task finished. Delegate a new task to resume the work.', workState: 'result_available', hasPendingChildRuns: false, endedAt: Date.now() });
        scope.records.set(record.task.taskId, record);
      }
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw new Error('Saved subagent tasks could not be read. Their file has been preserved.');
    }
    this.scopes.set(key, scope);
    if (request.threadId?.startsWith('agent-thread-') || [...scope.records.values()].some(record => record.task.status === 'interrupted' && !record.acknowledged)) await this.save(scope);
    return scope;
  }

  private projectDirectory(projectId: string): string {
    return path.join(this.options.directory, createHash('sha256').update(projectId).digest('hex'));
  }

  private async save(scope: Scope): Promise<void> {
    // Retain bounded history, but never evict active/unacknowledged work.
    const records = [...scope.records.values()];
    for (const record of records.slice(0, Math.max(0, records.length - 512))) {
      if (terminal(record.task) && record.acknowledged) scope.records.delete(record.task.taskId);
    }
    const data = JSON.stringify({ version: 1, rootThreadId: scope.rootRequest.threadId, records: [...scope.records.values()] });
    const saving = scope.saving.then(async () => {
      await mkdir(path.dirname(scope.file), { recursive: true });
      await writeFile(scope.file + '.tmp', data, { mode: 0o600 });
      await rename(scope.file + '.tmp', scope.file);
    });
    scope.saving = saving.catch(() => undefined);
    await saving;
  }
}
