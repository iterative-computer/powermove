import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process';
import readline from 'node:readline';

import type { ChatGPTAccountStatus, CodexModelOption, ReasoningEffort } from '../../shared/ipc';
import { isRecord, isString } from '../../shared/guards';
import { discoverCodexBinary } from './env';
import { isolatedCodexEnvironment, prepareIsolatedCodexHome } from './isolation';

const REQUEST_TIMEOUT_MS = 15_000;
const LOGIN_TIMEOUT_MS = 10 * 60_000;
const MAX_DETAIL_CHARS = 500;
const MODEL_ID = /^[a-zA-Z0-9][a-zA-Z0-9._:/-]{0,127}$/;
const EFFORTS: readonly ReasoningEffort[] = ['low', 'medium', 'high', 'xhigh', 'max'];

type JsonRpcId = number;

interface PendingRequest {
  resolve(value: unknown): void;
  reject(error: Error): void;
  timer: NodeJS.Timeout;
}

interface AppServerDependencies {
  discoverBinary(preference: string | null): Promise<string>;
  prepareHome(userData: string): Promise<string>;
  spawnProcess(
    binary: string,
    args: readonly string[],
    options: { env: NodeJS.ProcessEnv; stdio: ['pipe', 'pipe', 'pipe']; windowsHide: true }
  ): ChildProcessWithoutNullStreams;
  openExternal(url: string): Promise<void>;
  requestTimeoutMs: number;
  loginTimeoutMs: number;
}

const disconnected = (detail: string | null = null): ChatGPTAccountStatus => ({
  state: 'disconnected',
  email: null,
  planType: null,
  detail
});

const unavailable = (detail: string): ChatGPTAccountStatus => ({
  state: 'unavailable',
  email: null,
  planType: null,
  detail: detail.slice(0, MAX_DETAIL_CHARS)
});

function errorMessage(error: unknown): string {
  if (error instanceof Error && error.message.trim()) return error.message.trim();
  return String(error || 'Codex App Server is unavailable.').slice(0, MAX_DETAIL_CHARS);
}

function allowedAuthUrl(raw: string): string {
  const url = new URL(raw);
  const host = url.hostname.toLowerCase();
  const trusted = host === 'chatgpt.com' || host.endsWith('.chatgpt.com') ||
    host === 'openai.com' || host.endsWith('.openai.com');
  if (url.protocol !== 'https:' || !trusted) {
    throw new Error('Codex returned an unsafe ChatGPT authentication URL.');
  }
  return url.toString();
}

function parseAccountStatus(value: unknown): ChatGPTAccountStatus {
  if (!isRecord(value)) return disconnected();
  const account = value.account;
  if (!isRecord(account) || account.type !== 'chatgpt') return disconnected();
  return {
    state: 'connected',
    email: isString(account.email, 320) ? account.email : null,
    planType: isString(account.planType, 80) ? account.planType : null,
    detail: null
  };
}

function parseModels(value: unknown): CodexModelOption[] {
  if (!isRecord(value) || !Array.isArray(value.data)) return [];
  const models: CodexModelOption[] = [];
  for (const entry of value.data) {
    if (!isRecord(entry) || entry.hidden === true) continue;
    const id = typeof entry.model === 'string' ? entry.model : entry.id;
    if (typeof id !== 'string' || !MODEL_ID.test(id) || models.some(model => model.id === id)) continue;
    const efforts = Array.isArray(entry.supportedReasoningEfforts)
      ? entry.supportedReasoningEfforts
          .map(item => isRecord(item) ? item.reasoningEffort : null)
          .filter((item): item is ReasoningEffort => EFFORTS.includes(item as ReasoningEffort))
      : [];
    models.push({
      id,
      label: isString(entry.displayName, 120) ? entry.displayName : id,
      reasoningEfforts: [...new Set(efforts)]
    });
  }
  return models;
}

export class ChatGPTAccountClient {
  private readonly deps: AppServerDependencies;
  private child: ChildProcessWithoutNullStreams | null = null;
  private ready: Promise<void> | null = null;
  private nextId = 1;
  private readonly pending = new Map<JsonRpcId, PendingRequest>();
  private readonly listeners = new Set<(status: ChatGPTAccountStatus) => void>();
  private current: ChatGPTAccountStatus = {
    state: 'checking', email: null, planType: null, detail: null
  };
  private stderr = '';
  private activeLoginId: string | null = null;
  private loginTimer: NodeJS.Timeout | null = null;

  constructor(
    private readonly options: {
      userData: string;
      codexBinaryPref(): string | null;
      openExternal(url: string): Promise<void>;
    },
    dependencies: Partial<AppServerDependencies> = {}
  ) {
    this.deps = {
      discoverBinary: discoverCodexBinary,
      prepareHome: prepareIsolatedCodexHome,
      spawnProcess: (binary, args, spawnOptions) => spawn(binary, [...args], spawnOptions),
      openExternal: options.openExternal,
      requestTimeoutMs: REQUEST_TIMEOUT_MS,
      loginTimeoutMs: LOGIN_TIMEOUT_MS,
      ...dependencies
    };
  }

  onChanged(listener: (status: ChatGPTAccountStatus) => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  async status(): Promise<ChatGPTAccountStatus> {
    if (this.current.state === 'connecting') return this.current;
    try {
      await this.ensureStarted();
      const result = await this.request('account/read', { refreshToken: false });
      return this.publish(parseAccountStatus(result));
    } catch (error) {
      return this.publish(unavailable(errorMessage(error)));
    }
  }

  /** The app server returns the picker-visible models for this account. */
  async models(): Promise<CodexModelOption[]> {
    const models: CodexModelOption[] = [];
    const seenCursors = new Set<string>();
    let cursor: string | null = null;
    for (let page = 0; page < 5; page++) {
      const result = await this.request('model/list', {
        limit: 100,
        includeHidden: false,
        ...(cursor ? { cursor } : {})
      });
      for (const model of parseModels(result)) {
        if (!models.some(item => item.id === model.id)) models.push(model);
      }
      if (!isRecord(result) || !isString(result.nextCursor, 500) || seenCursors.has(result.nextCursor)) break;
      cursor = result.nextCursor;
      seenCursors.add(cursor);
    }
    return models;
  }

  async connect(): Promise<ChatGPTAccountStatus> {
    if (this.current.state === 'connecting' && this.activeLoginId !== null) return this.current;
    const existing = await this.status();
    if (existing.state === 'connected' || existing.state === 'connecting') return existing;
    if (existing.state === 'unavailable') return existing;

    let loginId: string | null = null;
    try {
      const result = await this.request('account/login/start', {
        type: 'chatgpt',
        useHostedLoginSuccessPage: true,
        appBrand: 'chatgpt'
      });
      if (
        !isRecord(result) ||
        result.type !== 'chatgpt' ||
        !isString(result.loginId, 200) ||
        !isString(result.authUrl, 8_000)
      ) {
        throw new Error('Codex did not return a ChatGPT sign-in URL.');
      }
      loginId = result.loginId;
      const authUrl = allowedAuthUrl(result.authUrl);
      this.beginLogin(loginId);
      await this.deps.openExternal(authUrl);
      return this.publish({
        state: 'connecting',
        email: null,
        planType: null,
        detail: 'Finish signing in in your browser.'
      });
    } catch (error) {
      if (loginId !== null) await this.cancelLogin(loginId);
      return this.publish(disconnected(errorMessage(error)));
    }
  }

  async disconnect(): Promise<ChatGPTAccountStatus> {
    try {
      await this.ensureStarted();
      if (this.activeLoginId !== null) await this.cancelLogin(this.activeLoginId);
      await this.request('account/logout', {});
      return this.publish(disconnected());
    } catch (error) {
      return this.publish(unavailable(errorMessage(error)));
    }
  }

  async shutdown(): Promise<void> {
    const child = this.child;
    this.child = null;
    this.ready = null;
    this.clearLogin();
    this.failPending(new Error('Codex App Server stopped.'));
    if (child && !child.killed) child.kill();
  }

  private publish(status: ChatGPTAccountStatus): ChatGPTAccountStatus {
    if (status.state === 'connected') this.clearLogin();
    this.current = status;
    for (const listener of this.listeners) listener(status);
    return status;
  }

  private async ensureStarted(): Promise<void> {
    if (this.ready !== null) return this.ready;
    this.ready = this.launch().catch((error) => {
      this.ready = null;
      throw error;
    });
    return this.ready;
  }

  private async launch(): Promise<void> {
    const [binary, runtimeHome] = await Promise.all([
      this.deps.discoverBinary(this.options.codexBinaryPref()),
      this.deps.prepareHome(this.options.userData)
    ]);
    const child = this.deps.spawnProcess(binary, ['app-server', '--stdio'], {
      env: isolatedCodexEnvironment(runtimeHome),
      stdio: ['pipe', 'pipe', 'pipe'],
      windowsHide: true
    });
    this.child = child;
    this.stderr = '';

    const lines = readline.createInterface({ input: child.stdout });
    lines.on('line', (line) => this.receive(line));
    child.stderr.setEncoding('utf8');
    child.stderr.on('data', (chunk: string) => {
      this.stderr = `${this.stderr}${chunk}`.slice(-4_000);
    });
    child.once('error', (error) => this.handleExit(error));
    child.once('exit', (code, signal) => {
      const suffix = this.stderr.trim() || `exit ${code ?? 'unknown'}${signal ? ` (${signal})` : ''}`;
      this.handleExit(new Error(`Codex App Server stopped: ${suffix}`));
    });

    await this.requestStarted('initialize', {
      clientInfo: { name: 'powermove', title: 'Powermove', version: '1.0.0' }
    });
    this.notify('initialized', {});
  }

  private async request(method: string, params: Record<string, unknown>): Promise<unknown> {
    await this.ensureStarted();
    return this.requestStarted(method, params);
  }

  private requestStarted(method: string, params: Record<string, unknown>): Promise<unknown> {
    const child = this.child;
    if (!child || child.killed || !child.stdin.writable) {
      return Promise.reject(new Error('Codex App Server is not running.'));
    }
    const id = this.nextId++;
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
    if (child && !child.killed && child.stdin.writable) {
      child.stdin.write(`${JSON.stringify({ method, params })}\n`);
    }
  }

  private receive(line: string): void {
    let message: unknown;
    try {
      message = JSON.parse(line);
    } catch {
      return;
    }
    if (!isRecord(message)) return;
    if (typeof message.id === 'number') {
      const pending = this.pending.get(message.id);
      if (!pending) return;
      this.pending.delete(message.id);
      clearTimeout(pending.timer);
      if (isRecord(message.error)) {
        const detail = isString(message.error.message, MAX_DETAIL_CHARS)
          ? message.error.message
          : 'Codex App Server request failed.';
        pending.reject(new Error(detail));
      } else {
        pending.resolve(message.result);
      }
      return;
    }
    if (!isString(message.method, 200)) return;
    if (message.method === 'account/updated' || message.method === 'account/login/completed') {
      if (message.method === 'account/login/completed' && isRecord(message.params)) {
        if (isString(message.params.loginId, 200) && message.params.loginId === this.activeLoginId) {
          this.clearLogin();
        }
      }
      if (message.method === 'account/login/completed' && isRecord(message.params) && message.params.success === false) {
        const detail = isString(message.params.error, MAX_DETAIL_CHARS)
          ? message.params.error
          : 'ChatGPT sign-in was not completed.';
        this.publish(disconnected(detail));
        return;
      }
      void this.refreshAfterNotification();
    }
  }

  private async refreshAfterNotification(): Promise<void> {
    try {
      const result = await this.request('account/read', { refreshToken: false });
      this.publish(parseAccountStatus(result));
    } catch (error) {
      this.publish(unavailable(errorMessage(error)));
    }
  }

  private handleExit(error: Error): void {
    if (this.child === null && this.ready === null) return;
    this.child = null;
    this.ready = null;
    this.clearLogin();
    this.failPending(error);
    this.publish(unavailable(error.message));
  }

  private failPending(error: Error): void {
    for (const request of this.pending.values()) {
      clearTimeout(request.timer);
      request.reject(error);
    }
    this.pending.clear();
  }

  private beginLogin(loginId: string): void {
    this.clearLogin();
    this.activeLoginId = loginId;
    this.loginTimer = setTimeout(() => {
      if (this.activeLoginId !== loginId) return;
      void this.cancelLogin(loginId).then(() => {
        this.publish(disconnected('ChatGPT sign-in timed out. Try connecting again.'));
      });
    }, this.deps.loginTimeoutMs);
  }

  private clearLogin(): void {
    if (this.loginTimer !== null) clearTimeout(this.loginTimer);
    this.loginTimer = null;
    this.activeLoginId = null;
  }

  private async cancelLogin(loginId: string): Promise<void> {
    if (this.activeLoginId === loginId) this.clearLogin();
    try {
      await this.request('account/login/cancel', { loginId });
    } catch {
      // The browser may have completed while cancellation was in flight.
    }
  }
}

export const chatGPTAccountStatusFromAppServer = parseAccountStatus;
export const isAllowedChatGPTAuthUrl = (url: string): boolean => {
  try { allowedAuthUrl(url); return true; } catch { return false; }
};
