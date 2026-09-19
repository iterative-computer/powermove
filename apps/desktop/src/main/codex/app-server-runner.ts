import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import readline from 'node:readline';

import type {
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
  if (isRecord(value) && isRecord(value.error) && isString(value.error.message, 4_000)) {
    return value.error.message;
  }
  return fallback;
}

function liveInspectionConfig(nativeTools: NativeMcpServerConfig): Record<string, unknown> {
  return {
    mcp_servers: {
      powermove: {
        command: nativeTools.command,
        args: nativeTools.args,
        env: nativeTools.env,
        startup_timeout_sec: 30,
        tool_timeout_sec: 120,
        required: true,
        enabled_tools: [...POWERMOVE_LIVE_INSPECTION_TOOL_NAMES],
        default_tools_approval_mode: 'approve'
      }
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
  if (type === 'mcptoolcall') {
    const rawTool = typeof item.tool === 'string' && item.tool ? item.tool : 'mcp';
    const detail = toolDetail(rawTool, { tool: rawTool });
    return {
      kind: 'tool-start', itemId, toolName: rawTool.slice(0, TRACE_ITEM_ID_CHARS), label: humanLabel(rawTool),
      ...(detail ? { detail } : {})
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
  if (!['commandexecution', 'filechange', 'mcptoolcall', 'websearch', 'imagegeneration', 'computeruse'].includes(type)) {
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
  else rawOutput = item.output;
  const output = outputExcerpt(rawOutput);
  return { kind: 'tool-end', itemId, isError, ...(output ? { output } : {}) };
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
      if (preparation.cancelled) return cancelled();
      directory = await mkdtemp(path.join(tmpdir(), 'powermove-codex-app-'));
      const input: Array<Record<string, string>> = [{ type: 'text', text: req.prompt }];
      for (const [index, image] of req.images.entries()) {
        const extension = image[0] === 0x89 && image[1] === 0x50 ? 'png' : 'jpg';
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
        sandbox: 'readOnly',
        ephemeral: true,
        ...(options.nativeTools ? { config: liveInspectionConfig(options.nativeTools) } : {}),
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
      const extension = image[0] === 0x89 && image[1] === 0x50 ? 'png' : 'jpg';
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
        clientInfo: { name: 'powermove_agent', title: 'Powermove Agent', version: '1.0.0' }
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

  private receive(line: string): void {
    let message: unknown;
    try { message = JSON.parse(line); } catch { return; }
    if (!isRecord(message)) return;
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
    const topLevelTurnId = isString(params.turnId, 200) ? params.turnId : null;
    const turnId = topLevelTurnId ?? (methodIs(method, 'turn/completed') && isRecord(params.turn) && isString(params.turn.id, 200)
      ? params.turn.id
      : null);
    if (threadId === null || turnId === null) return;
    const turn = [...this.active.values()].find((candidate) =>
      candidate.threadId === threadId && (candidate.turnId === null || candidate.turnId === turnId));
    if (!turn) return;

    if (methodIs(method, 'item/commandExecution/outputDelta')) return;

    if (methodIs(method, 'item/started') && isRecord(params.item)) {
      const trace = appServerToolStart(params.item);
      if (trace) turn.onTrace?.(trace);
      return;
    }
    if (methodIs(method, 'item/agentMessage/delta') && isString(params.delta)) {
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
      if (normalizedItemType(item.type) === 'agentmessage' && isString(item.text)) {
        turn.finalText = item.text;
        const itemId = traceItemId(item);
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
