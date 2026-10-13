import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import readline from 'node:readline';

import type {
  CodexAnswerRequest,
  CodexQuestion,
  CodexRunRequest,
  CodexRunResult,
  CodexSteerRequest,
  CodexTraceEvent
} from '../../shared/ipc';
import { isRecord, isString } from '../../shared/guards';
import { discoverCodexBinary } from './env';
import { isolatedCodexEnvironment, prepareIsolatedCodexHome } from './isolation';
import {
  POWERMOVE_LIVE_INSPECTION_TOOL_NAMES,
  type NativeMcpServerConfig
} from '../agent-tools/spec';
import { fragmentText, humanLabel, outputExcerpt, toolDetail } from '../agent-tools/trace-format';
import { mediaToolSubject } from '../../shared/media-tools';
import { imageExtension } from '../image-extension';
import { loadUserMcpServers, type UserMcpServers } from '../agent-tools/user-mcp';

const REQUEST_TIMEOUT_MS = 30_000;
const TURN_TIMEOUT_MS = 3_600_000;

interface RunCallbacks {
  onProgress?: (text: string) => void;
  onTrace?: (step: CodexTraceEvent) => void;
}

interface AppServerRunOptions extends RunCallbacks {
  userData: string;
  codexBinaryPref?: string | null;
  nativeTools?: NativeMcpServerConfig;
  additionalInstructions?: string;
  externalMcpServers?: UserMcpServers;
}

interface ActiveTurn extends RunCallbacks {
  requestId: string;
  threadId: string;
  turnId: string | null;
  finalText: string;
  directory: string;
  queuedSteering: CodexSteerRequest[];
  cancelRequested: boolean;
  agentMessageDeltaItemIds: Set<string>;
  sawUnscopedAgentMessageDelta: boolean;
  /** Held `request_user_input` calls: trace item id → App Server request id. */
  questions: Map<string, string | number>;
  /** Async question messages already shown as a question card. */
  questionMessageIds: Set<string>;
  resolve(result: CodexRunResult): void;
  timer: NodeJS.Timeout;
}

interface PendingRequest {
  resolve(value: unknown): void;
  reject(error: Error): void;
  timer: NodeJS.Timeout;
}

interface AppServerRunnerDependencies {
  discoverBinary(preference: string | null): Promise<string>;
  prepareHome(userData: string): Promise<string>;
  spawnProcess(binary: string, args: readonly string[], options: {
    env: NodeJS.ProcessEnv;
    stdio: ['pipe', 'pipe', 'pipe'];
    windowsHide: true;
  }): ChildProcessWithoutNullStreams;
  requestTimeoutMs: number;
  turnTimeoutMs: number;
}

function appServerError(value: unknown, fallback: string): string {
  // Codex compacts automatically; this only arrives once that could not help.
  if (isRecord(value) && isRecord(value.error) && value.error.codexErrorInfo === 'contextWindowExceeded') {
    return 'This conversation is too long for the model, even after compacting it. Start a new thread to continue.';
  }
  if (isRecord(value) && isRecord(value.error) && isString(value.error.message, 4_000)) {
    return value.error.message;
  }
  return fallback;
}

const COLLAB_LABELS: Readonly<Record<string, string>> = {
  spawnAgent: 'Start subagent', sendInput: 'Message subagent', sendMessage: 'Message subagent',
  followupTask: 'Message subagent', resumeAgent: 'Resume subagent', wait: 'Wait for subagents',
  closeAgent: 'Close subagent', interruptAgent: 'Stop subagent', listAgents: 'List subagents'
};

function liveInspectionConfig(nativeTools?: NativeMcpServerConfig, externalMcpServers: UserMcpServers = {}): Record<string, unknown> {
  return {
    mcp_servers: {
      ...Object.fromEntries(Object.entries(externalMcpServers).filter(([name]) => name !== 'powermove')),
      ...(nativeTools ? { powermove: {
        command: nativeTools.command,
        args: nativeTools.args,
        env: nativeTools.env,
        startup_timeout_sec: 30,
        tool_timeout_sec: 120,
        required: true,
        enabled_tools: [...POWERMOVE_LIVE_INSPECTION_TOOL_NAMES],
        default_tools_approval_mode: 'approve'
      } } : {})
    }
  };
}

const TRACE_ITEM_ID_CHARS = 120;

function normalizedItemType(value: unknown): string {
  return typeof value === 'string' ? value.replaceAll('_', '').toLowerCase() : '';
}

function basename(value: unknown): string {
  if (typeof value !== 'string') return '';
  return value.replace(/[\u0000-\u001f\u007f-\u009f]/g, '').split(/[\\/]/).filter(Boolean).at(-1) ?? '';
}

function changedFileNames(item: Record<string, unknown>): string[] {
  if (!Array.isArray(item.changes)) return [];
  return item.changes
    .map((change) => typeof change === 'string' ? basename(change) : isRecord(change) ? basename(change.path) : '')
    .filter(Boolean);
}

function traceItemId(item: Record<string, unknown>): string | null {
  return isString(item.id, TRACE_ITEM_ID_CHARS * 2) ? item.id.slice(0, TRACE_ITEM_ID_CHARS) : null;
}

function appServerToolStart(item: Record<string, unknown>): CodexTraceEvent | null {
  const itemId = traceItemId(item);
  if (!itemId) return null;
  const type = normalizedItemType(item.type);
  if (type === 'commandexecution') {
    const detail = toolDetail('Bash', { command: item.command });
    return { kind: 'tool-start', itemId, toolName: 'bash', label: 'Run', ...(detail ? { detail } : {}) };
  }
  if (type === 'filechange') {
    const detail = toolDetail('Edit', { file_path: changedFileNames(item).join(', ') });
    return { kind: 'tool-start', itemId, toolName: 'edit', label: 'Edit', ...(detail ? { detail } : {}) };
  }
  if (type === 'websearch') {
    const detail = toolDetail('WebSearch', { query: item.query });
    return { kind: 'tool-start', itemId, toolName: 'search', label: 'Search', ...(detail ? { detail } : {}) };
  }
  if (type === 'imagegeneration') {
    return { kind: 'tool-start', itemId, toolName: 'image', label: 'Image' };
  }
  if (type === 'computeruse') {
    return { kind: 'tool-start', itemId, toolName: 'computer', label: 'Computer' };
  }
  if (type === 'contextcompaction') {
    return { kind: 'tool-start', itemId, toolName: 'compact', label: 'Compacting context' };
  }
  if (type === 'imageview') {
    const detail = basename(item.path);
    return { kind: 'tool-start', itemId, toolName: 'view_image', label: 'View image', ...(detail ? { detail } : {}) };
  }
  if (type === 'sleep') {
    const seconds = typeof item.durationMs === 'number' ? Math.round(item.durationMs / 1000) : 0;
    return { kind: 'tool-start', itemId, toolName: 'wait', label: 'Wait', ...(seconds > 0 ? { detail: `${seconds}s` } : {}) };
  }
  if (type === 'collabagenttoolcall') {
    const detail = toolDetail('Task', { description: item.prompt });
    return {
      kind: 'tool-start', itemId, toolName: 'agent',
      label: (typeof item.tool === 'string' && COLLAB_LABELS[item.tool]) || 'Subagent',
      ...(detail ? { detail } : {})
    };
  }
  if (type === 'mcptoolcall' || type === 'dynamictoolcall') {
    const rawTool = typeof item.tool === 'string' && item.tool ? item.tool : 'mcp';
    const detail = toolDetail(rawTool, { tool: rawTool });
    const subject = mediaToolSubject(rawTool, item.arguments);
    return {
      kind: 'tool-start', itemId, toolName: rawTool.slice(0, TRACE_ITEM_ID_CHARS), label: humanLabel(rawTool),
      ...(detail ? { detail } : {}), ...(subject ? { subject } : {})
    };
  }
  return null;
}

function appServerErrorText(item: Record<string, unknown>): string {
  if (typeof item.error === 'string') return item.error;
  if (isRecord(item.error) && typeof item.error.message === 'string') return item.error.message;
  return typeof item.status === 'string' ? item.status : '';
}

function appServerToolEnd(item: Record<string, unknown>): CodexTraceEvent | null {
  const itemId = traceItemId(item);
  if (!itemId) return null;
  const type = normalizedItemType(item.type);
  if (![
    'commandexecution', 'filechange', 'mcptoolcall', 'dynamictoolcall', 'websearch', 'imagegeneration', 'computeruse',
    'contextcompaction', 'imageview', 'sleep', 'collabagenttoolcall'
  ].includes(type)) {
    return null;
  }
  const status = typeof item.status === 'string' ? item.status.toLowerCase() : '';
  const isError = status === 'failed' || status === 'error' || status === 'declined';
  let rawOutput: unknown;
  if (isError) rawOutput = appServerErrorText(item);
  else if (type === 'commandexecution') rawOutput = item.aggregatedOutput ?? item.aggregated_output ?? item.output;
  else if (type === 'filechange') {
    const files = changedFileNames(item);
    rawOutput = files.length > 3 ? `${files.length} files changed` : files.join(', ');
  } else if (type === 'websearch') rawOutput = item.query;
  else if (type === 'mcptoolcall') rawOutput = item.result ?? item.output;
  else if (type === 'dynamictoolcall') rawOutput = Array.isArray(item.contentItems)
    ? item.contentItems.map((part) => isRecord(part) && typeof part.text === 'string' ? part.text : '').join('\n')
    : undefined;
  else if (type === 'contextcompaction') rawOutput = 'Earlier context summarized';
  else if (type === 'imageview' || type === 'sleep' || type === 'collabagenttoolcall') rawOutput = undefined;
  else rawOutput = item.output;
  const output = outputExcerpt(rawOutput);
  return { kind: 'tool-end', itemId, isError, ...(output ? { output } : {}) };
}

const QUESTION_LIMITS = { questions: 6, options: 8, header: 80, question: 1_000, label: 160, description: 400 } as const;

function clip(value: unknown, limit: number): string {
  return typeof value === 'string' ? value.replace(/[\u0000-\u0008\u000b-\u001f\u007f]/g, '').trim().slice(0, limit) : '';
}

/** `item/tool/requestUserInput` questions, bounded for the renderer. */
function replyQuestions(value: unknown): CodexQuestion[] {
  if (!Array.isArray(value)) return [];
  return value.slice(0, QUESTION_LIMITS.questions).flatMap((entry): CodexQuestion[] => {
    if (!isRecord(entry) || !isString(entry.id, 200)) return [];
    const question = clip(entry.question, QUESTION_LIMITS.question);
    if (!question) return [];
    const options = (Array.isArray(entry.options) ? entry.options : []).slice(0, QUESTION_LIMITS.options).flatMap((option) => {
      const label = isRecord(option) ? clip(option.label, QUESTION_LIMITS.label) : '';
      return label ? [{ label, description: clip((option as Record<string, unknown>).description, QUESTION_LIMITS.description) }] : [];
    });
    return [{
      id: entry.id,
      header: clip(entry.header, QUESTION_LIMITS.header),
      question,
      options,
      // With no options a typed answer is the only possible answer.
      allowOther: entry.isOther === true || options.length === 0,
      secret: entry.isSecret === true
    }];
  });
}

/** Questions posted on an async agent message (`title` + plain options). */
function messageQuestions(itemId: string, value: unknown): CodexQuestion[] {
  if (!Array.isArray(value)) return [];
  return value.slice(0, QUESTION_LIMITS.questions).flatMap((entry, index): CodexQuestion[] => {
    const question = isRecord(entry) ? clip(entry.title, QUESTION_LIMITS.question) : '';
    if (!question) return [];
    const options = (Array.isArray((entry as Record<string, unknown>).options) ? (entry as Record<string, unknown>).options as unknown[] : [])
      .slice(0, QUESTION_LIMITS.options)
      .map((option) => clip(option, QUESTION_LIMITS.label))
      .filter(Boolean)
      .map((label) => ({ label, description: '' }));
    // The async question tool always accepts a typed answer.
    return [{ id: `${itemId}-${index}`, header: '', question, options, allowOther: true, secret: false }];
  });
}

function methodIs(method: string, suffix: string): boolean {
  return method === suffix || method.endsWith(`/${suffix}`);
}

/**
 * Rich Codex transport used by the editor agent. Unlike `codex exec`, App
 * Server exposes the active thread and turn ids required by `turn/steer`.
 */
export class CodexAppServerRunner {
  private readonly deps: AppServerRunnerDependencies;
  private child: ChildProcessWithoutNullStreams | null = null;
  private ready: Promise<void> | null = null;
  private nextRpcId = 1;
  private readonly pending = new Map<number, PendingRequest>();
  private readonly active = new Map<string, ActiveTurn>();
  private stderr = '';
  private readonly preparing = new Map<string, { cancelled: boolean }>();

  constructor(dependencies: Partial<AppServerRunnerDependencies> = {}) {
    this.deps = {
      discoverBinary: discoverCodexBinary,
      prepareHome: prepareIsolatedCodexHome,
      spawnProcess: (binary, args, options) => spawn(binary, [...args], options),
      requestTimeoutMs: REQUEST_TIMEOUT_MS,
      turnTimeoutMs: TURN_TIMEOUT_MS,
      ...dependencies
    };
  }

  async run(
    req: CodexRunRequest,
    options: AppServerRunOptions
  ): Promise<CodexRunResult> {
    if (req.mode !== 'editor') return { ok: false, error: 'App Server editor transport received a non-editor run.', cancelled: false };
    if (this.active.has(req.id) || this.preparing.has(req.id)) return { ok: false, error: `A Codex run with id ${req.id} is already active.`, cancelled: false };

    const preparation = { cancelled: false };
    this.preparing.set(req.id, preparation);
    let directory: string | null = null;
    const cancelled = (): CodexRunResult => ({ ok: false, error: 'The Codex run was cancelled.', cancelled: true });
    try {
      await this.ensureStarted(options.userData, options.codexBinaryPref ?? null);
      const externalMcpServers = options.externalMcpServers ?? await loadUserMcpServers('chatgpt');
      if (preparation.cancelled) return cancelled();
      directory = await mkdtemp(path.join(tmpdir(), 'powermove-codex-app-'));
      const input: Array<Record<string, string>> = [{ type: 'text', text: req.prompt }];
      for (const [index, image] of req.images.entries()) {
        const extension = imageExtension(image);
        const imagePath = path.join(directory, `frame-${index}.${extension}`);
        await writeFile(imagePath, image);
        input.push({ type: 'localImage', path: imagePath });
      }

      /* The renderer prompt already carries Powermove's bounded conversation
         history. A fresh ephemeral App Server thread prevents that history
         from being duplicated across turns while still enabling live steering
         inside this run. */
      if (preparation.cancelled) return cancelled();
      const startedThread = await this.request('thread/start', {
        cwd: directory,
        approvalPolicy: 'never',
        sandbox: 'read-only',
        ephemeral: true,
        config: { ...liveInspectionConfig(options.nativeTools, externalMcpServers), ...(options.additionalInstructions ? { developer_instructions: options.additionalInstructions } : {}) },
        ...(req.model ? { model: req.model } : {}),
        serviceName: 'powermove'
      });
      if (!isRecord(startedThread) || !isRecord(startedThread.thread) || !isString(startedThread.thread.id, 200)) {
        throw new Error('Codex App Server did not return a thread id.');
      }
      const threadId = startedThread.thread.id;
      if (preparation.cancelled) return cancelled();
      const turnDirectory = directory;

      return await new Promise<CodexRunResult>((resolve) => {
        const timer = setTimeout(() => {
          turn.cancelRequested = true;
          if (turn.turnId !== null) {
            void this.request('turn/interrupt', { threadId, turnId: turn.turnId }).catch(() => undefined);
          }
          this.finish(turn, { ok: false, error: 'The coding agent took too long to respond.', cancelled: false });
        }, this.deps.turnTimeoutMs);
        timer.unref();
        const turn: ActiveTurn = {
          requestId: req.id,
          threadId,
          turnId: null,
          finalText: '',
          directory: turnDirectory,
          queuedSteering: [],
          cancelRequested: false,
          agentMessageDeltaItemIds: new Set(),
          sawUnscopedAgentMessageDelta: false,
          questions: new Map(),
          questionMessageIds: new Set(),
          resolve,
          timer,
          onProgress: options.onProgress,
          onTrace: options.onTrace
        };
        this.active.set(req.id, turn);
        void this.request('turn/start', {
          threadId,
          input,
          cwd: directory,
          approvalPolicy: 'never',
          sandboxPolicy: { type: 'readOnly' },
          ...(req.model ? { model: req.model } : {}),
          ...(req.reasoningEffort ? { effort: req.reasoningEffort } : {}),
          outputSchema: req.schema ?? { type: 'object' }
        }).then(async (started) => {
          if (!isRecord(started) || !isRecord(started.turn) || !isString(started.turn.id, 200)) {
            if (!this.active.has(req.id)) return;
            this.finish(turn, { ok: false, error: 'Codex App Server did not return a turn id.', cancelled: false });
            return;
          }
          turn.turnId = started.turn.id;
          if (turn.cancelRequested) {
            await this.request('turn/interrupt', { threadId: turn.threadId, turnId: turn.turnId }).catch(() => undefined);
            return;
          }
          if (!this.active.has(req.id)) return;
          for (const steering of turn.queuedSteering.splice(0)) await this.sendSteering(turn, steering);
        }).catch((error) => {
          this.finish(turn, { ok: false, error: error instanceof Error ? error.message : String(error), cancelled: false });
        });
      });
    } catch (error) {
      if (preparation.cancelled) return cancelled();
      return { ok: false, error: error instanceof Error ? error.message : String(error), cancelled: false };
    } finally {
      this.preparing.delete(req.id);
      if (directory) await rm(directory, { recursive: true, force: true }).catch(() => undefined);
    }
  }

  async steer(req: CodexSteerRequest): Promise<boolean> {
    const turn = this.active.get(req.id);
    if (!turn) return false;
    if (turn.turnId === null) {
      turn.queuedSteering.push(req);
      return true;
    }
    await this.sendSteering(turn, req);
    return true;
  }

  /** Settles a held `request_user_input` call with the user's answers. */
  answer(req: CodexAnswerRequest): boolean {
    const turn = this.active.get(req.id);
    const rpcId = turn?.questions.get(req.itemId);
    if (!turn || rpcId === undefined) return false;
    turn.questions.delete(req.itemId);
    this.respond(rpcId, {
      answers: Object.fromEntries(Object.entries(req.answers).map(([id, answers]) => [id, { answers }]))
    });
    return true;
  }

  async cancel(id: string): Promise<boolean> {
    const preparation = this.preparing.get(id);
    if (preparation) preparation.cancelled = true;
    const turn = this.active.get(id);
    if (!turn) return Boolean(preparation);
    turn.cancelRequested = true;
    if (turn.turnId !== null) {
      try {
        await this.request('turn/interrupt', { threadId: turn.threadId, turnId: turn.turnId });
      } catch {
        // The turn may have completed while interruption was in flight.
      }
    }
    this.finish(turn, { ok: false, error: 'The Codex run was cancelled.', cancelled: true });
    return true;
  }

  async cancelAll(): Promise<void> {
    await Promise.all([...new Set([...this.preparing.keys(), ...this.active.keys()])].map((id) => this.cancel(id)));
  }

  async shutdown(): Promise<void> {
    await this.cancelAll();
    const child = this.child;
    this.child = null;
    this.ready = null;
    this.failPending(new Error('Codex App Server stopped.'));
    if (child && !child.killed) child.kill();
  }

  private async sendSteering(turn: ActiveTurn, req: CodexSteerRequest): Promise<void> {
    if (turn.turnId === null) return;
    const input: Array<Record<string, string>> = [{ type: 'text', text: req.prompt }];
    for (const [index, image] of req.images.entries()) {
      const extension = imageExtension(image);
      const imagePath = path.join(turn.directory, `steer-${Date.now()}-${index}.${extension}`);
      await writeFile(imagePath, image);
      input.push({ type: 'localImage', path: imagePath });
    }
    await this.request('turn/steer', {
      threadId: turn.threadId,
      expectedTurnId: turn.turnId,
      input
    });
  }

  private finish(turn: ActiveTurn, result: CodexRunResult): void {
    if (this.active.get(turn.requestId) !== turn) return;
    this.active.delete(turn.requestId);
    clearTimeout(turn.timer);
    // An unanswered question must not keep App Server waiting on this client.
    for (const rpcId of turn.questions.values()) this.respond(rpcId, { answers: {} });
    turn.questions.clear();
    turn.resolve(result);
  }

  private async ensureStarted(userData: string, preference: string | null): Promise<void> {
    if (this.ready !== null) return this.ready;
    this.ready = this.launch(userData, preference).catch((error) => {
      this.ready = null;
      throw error;
    });
    return this.ready;
  }

  private async launch(userData: string, preference: string | null): Promise<void> {
    const [binary, runtimeHome] = await Promise.all([
      this.deps.discoverBinary(preference),
      this.deps.prepareHome(userData)
    ]);
    const child = this.deps.spawnProcess(binary, ['app-server', '--stdio'], {
      env: isolatedCodexEnvironment(runtimeHome),
      stdio: ['pipe', 'pipe', 'pipe'],
      windowsHide: true
    });
    this.child = child;
    this.stderr = '';
    readline.createInterface({ input: child.stdout }).on('line', (line) => {
      if (this.child === child) this.receive(line);
    });
    child.stderr.setEncoding('utf8');
    child.stderr.on('data', (chunk: string) => { if (this.child === child) this.stderr = `${this.stderr}${chunk}`.slice(-4_000); });
    child.once('error', (error) => { if (this.child === child) this.handleExit(error); });
    child.stdin.on('error', (error) => { if (this.child === child) this.handleExit(error); });
    child.once('exit', (code, signal) => {
      if (this.child === child) this.handleExit(new Error(
        `Codex App Server stopped: ${this.stderr.trim() || `exit ${code ?? 'unknown'}${signal ? ` (${signal})` : ''}`}`
      ));
    });
    try {
      await this.requestStarted('initialize', {
        clientInfo: { name: 'powermove_agent', title: 'Powermove Agent', version: '1.0.0' },
        // Agent questions (`request_user_input`, async message questions) are
        // experimental App Server surface and are withheld without this opt-in.
        capabilities: { experimentalApi: true, requestAttestation: false }
      });
      this.notify('initialized', {});
    } catch (error) {
      if (this.child === child) this.handleExit(error instanceof Error ? error : new Error(String(error)));
      if (!child.killed) child.kill();
      throw error;
    }
  }

  private async request(method: string, params: Record<string, unknown>): Promise<unknown> {
    if (this.ready !== null) await this.ready;
    return this.requestStarted(method, params);
  }

  private requestStarted(method: string, params: Record<string, unknown>): Promise<unknown> {
    const child = this.child;
    if (!child || child.killed || !child.stdin.writable) return Promise.reject(new Error('Codex App Server is not running.'));
    const id = this.nextRpcId++;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`Codex App Server timed out while handling ${method}.`));
      }, this.deps.requestTimeoutMs);
      this.pending.set(id, { resolve, reject, timer });
      child.stdin.write(`${JSON.stringify({ method, id, params })}\n`);
    });
  }

  private notify(method: string, params: Record<string, unknown>): void {
    const child = this.child;
    if (child && !child.killed && child.stdin.writable) child.stdin.write(`${JSON.stringify({ method, params })}\n`);
  }

  private respond(id: string | number, result: unknown): void {
    const child = this.child;
    if (child && !child.killed && child.stdin.writable) child.stdin.write(`${JSON.stringify({ id, result })}\n`);
  }

  private respondError(id: string | number, message: string): void {
    const child = this.child;
    if (child && !child.killed && child.stdin.writable) {
      child.stdin.write(`${JSON.stringify({ id, error: { code: -32601, message } })}\n`);
    }
  }

  private turnFor(threadId: string | null, turnId: string | null): ActiveTurn | undefined {
    if (threadId === null) return undefined;
    return [...this.active.values()].find((candidate) =>
      candidate.threadId === threadId && (turnId === null || candidate.turnId === null || candidate.turnId === turnId));
  }

  /* App Server asks the client things mid-turn. Every request gets a reply —
     an unanswered one stalls the turn until it times out. */
  private handleServerRequest(id: string | number, method: string, params: Record<string, unknown>): void {
    const turn = this.turnFor(
      isString(params.threadId, 200) ? params.threadId : null,
      isString(params.turnId, 200) ? params.turnId : null
    );
    if (methodIs(method, 'item/tool/requestUserInput')) {
      const questions = replyQuestions(params.questions);
      const itemId = isString(params.itemId, TRACE_ITEM_ID_CHARS * 2) ? params.itemId.slice(0, TRACE_ITEM_ID_CHARS) : null;
      if (!turn || !itemId || !questions.length) {
        this.respond(id, { answers: {} });
        return;
      }
      turn.questions.set(itemId, id);
      turn.onTrace?.({ kind: 'question', itemId, questions, transport: 'reply', blocking: params.isBlocking !== false });
      return;
    }
    if (methodIs(method, 'mcpServer/elicitation/request')) {
      this.respond(id, { action: 'decline', content: null, _meta: null });
      return;
    }
    // Approvals cannot arrive under `approvalPolicy: never`; anything else is
    // a capability this client does not offer.
    this.respondError(id, `Powermove does not handle ${method}.`);
  }

  private receive(line: string): void {
    let message: unknown;
    try { message = JSON.parse(line); } catch { return; }
    if (!isRecord(message)) return;
    // A server request carries an id too; it must never settle one of ours.
    if ((typeof message.id === 'number' || isString(message.id, 200)) && isString(message.method, 200)) {
      this.handleServerRequest(message.id, message.method, isRecord(message.params) ? message.params : {});
      return;
    }
    if (typeof message.id === 'number') {
      const pending = this.pending.get(message.id);
      if (!pending) return;
      this.pending.delete(message.id);
      clearTimeout(pending.timer);
      if (isRecord(message.error)) pending.reject(new Error(appServerError(message, 'Codex App Server request failed.')));
      else pending.resolve(message.result);
      return;
    }
    if (!isString(message.method, 200) || !isRecord(message.params)) return;
    const method = message.method;
    const params = message.params;
    const threadId = isString(params.threadId, 200) ? params.threadId : null;
    if (methodIs(method, 'warning') && isString(params.message, 2_000)) {
      const text = params.message.replace(/\s+/g, ' ').trim().slice(0, 320);
      for (const candidate of this.active.values()) {
        if (text && (threadId === null || candidate.threadId === threadId)) candidate.onProgress?.(text);
      }
      return;
    }
    if (methodIs(method, 'serverRequest/resolved')) {
      const requestId = params.requestId;
      for (const candidate of this.active.values()) {
        if (candidate.threadId !== threadId) continue;
        for (const [itemId, rpcId] of candidate.questions) {
          if (rpcId !== requestId) continue;
          candidate.questions.delete(itemId);
          candidate.onTrace?.({ kind: 'question-closed', itemId });
        }
      }
      return;
    }
    const topLevelTurnId = isString(params.turnId, 200) ? params.turnId : null;
    const turnId = topLevelTurnId ?? (methodIs(method, 'turn/completed') && isRecord(params.turn) && isString(params.turn.id, 200)
      ? params.turn.id
      : null);
    if (threadId === null || turnId === null) return;
    const turn = this.turnFor(threadId, turnId);
    if (!turn) return;

    if (methodIs(method, 'item/commandExecution/outputDelta')) return;

    /* Provider housekeeping the person should see without it being mistaken
       for the agent's own words: retries, model reroutes, warnings. */
    if (methodIs(method, 'error') && isRecord(params.error)) {
      if (params.willRetry === true) turn.onProgress?.('Connection interrupted. Retrying…');
      return;
    }
    if (methodIs(method, 'model/rerouted') && isString(params.toModel, 120)) {
      turn.onProgress?.(`Continuing on ${params.toModel}`);
      return;
    }

    if (methodIs(method, 'item/started') && isRecord(params.item)) {
      if (this.questionMessage(turn, params.item)) return;
      const trace = appServerToolStart(params.item);
      if (trace) turn.onTrace?.(trace);
      return;
    }
    if (methodIs(method, 'item/agentMessage/delta') && isString(params.delta)) {
      // A question's text is shown by its card, not repeated as prose.
      if (isString(params.itemId, TRACE_ITEM_ID_CHARS * 2)
        && turn.questionMessageIds.has(params.itemId.slice(0, TRACE_ITEM_ID_CHARS))) return;
      const text = fragmentText(params.delta);
      if (text) turn.onTrace?.({ kind: 'answer', text });
      if (isString(params.itemId, TRACE_ITEM_ID_CHARS * 2)) {
        turn.agentMessageDeltaItemIds.add(params.itemId.slice(0, TRACE_ITEM_ID_CHARS));
      } else {
        turn.sawUnscopedAgentMessageDelta = true;
      }
      return;
    }
    if ((methodIs(method, 'item/reasoning/textDelta') || methodIs(method, 'item/reasoning/summaryTextDelta')) && isString(params.delta)) {
      const text = fragmentText(params.delta);
      if (text) turn.onTrace?.({ kind: 'thought', text });
      if (methodIs(method, 'item/reasoning/summaryTextDelta')) {
        const progress = params.delta.replace(/\s+/g, ' ').trim().slice(0, 320);
        if (progress) turn.onProgress?.(progress);
      }
      return;
    }
    if (methodIs(method, 'item/completed') && isRecord(params.item)) {
      const item = params.item;
      const toolTrace = appServerToolEnd(item);
      if (toolTrace) {
        turn.onTrace?.(toolTrace);
        return;
      }
      if (this.questionMessage(turn, item)) return;
      if (normalizedItemType(item.type) === 'agentmessage' && isString(item.text)) {
        const itemId = traceItemId(item);
        /* An async message (a mid-run update) is never the structured final
           answer, even when it lands last before the JSON. */
        if (item.delivery !== 'async') turn.finalText = item.text;
        const alreadyStreamed = turn.sawUnscopedAgentMessageDelta || (itemId !== null && turn.agentMessageDeltaItemIds.has(itemId));
        if (!alreadyStreamed) {
          const text = fragmentText(item.text);
          if (text) turn.onTrace?.({ kind: 'answer', text });
        }
      }
      return;
    }
    if (!methodIs(method, 'turn/completed') || !isRecord(params.turn)) return;
    const status = params.turn.status;
    if (status === 'completed') {
      this.finish(turn, turn.finalText
        ? { ok: true, text: turn.finalText, access: 'editor' }
        : { ok: false, error: 'ChatGPT generation completed without a result.', cancelled: false });
    } else if (status === 'interrupted') {
      this.finish(turn, { ok: false, error: 'The Codex run was cancelled.', cancelled: true });
    } else {
      this.finish(turn, { ok: false, error: appServerError(params.turn, 'ChatGPT generation failed.'), cancelled: false });
    }
  }

  /* `request_user_input_async` arrives as an agent message carrying
     `questions`. The model keeps working; the card is its only rendering, so
     its text (the question plus a bulleted option list) is not also prose. */
  private questionMessage(turn: ActiveTurn, item: Record<string, unknown>): boolean {
    if (normalizedItemType(item.type) !== 'agentmessage') return false;
    const itemId = traceItemId(item);
    if (!itemId) return false;
    if (turn.questionMessageIds.has(itemId)) return true;
    const questions = messageQuestions(itemId, item.questions);
    if (!questions.length) return false;
    turn.questionMessageIds.add(itemId);
    turn.onTrace?.({ kind: 'question', itemId, questions, transport: 'message', blocking: false });
    return true;
  }

  private handleExit(error: Error): void {
    if (this.child === null && this.ready === null) return;
    this.child = null;
    this.ready = null;
    this.failPending(error);
    for (const turn of [...this.active.values()]) {
      this.finish(turn, { ok: false, error: error.message, cancelled: false });
    }
  }

  private failPending(error: Error): void {
    for (const pending of this.pending.values()) {
      clearTimeout(pending.timer);
      pending.reject(error);
    }
    this.pending.clear();
  }
}
