import type { ChildProcess, SpawnOptions } from 'node:child_process';
import { query, type ModelInfo } from '@anthropic-ai/claude-agent-sdk';
import { spawnClaudeProcess } from './process';

import type { ClaudeAccountStatus, ClaudeModelOption, ReasoningEffort } from '../../shared/ipc';
import { isRecord, isString } from '../../shared/guards';
import { discoverClaudeBinary } from './env';
import { isolatedClaudeEnvironment, prepareIsolatedClaudeHome } from './isolation';

const STATUS_TIMEOUT_MS = 15_000;
const LOGIN_TIMEOUT_MS = 10 * 60_000;
const MAX_DETAIL_CHARS = 500;
const MODEL_TIMEOUT_MS = 30_000;
const MODEL_ID = /^[a-zA-Z0-9][a-zA-Z0-9._:/\[\]-]{0,127}$/;
const EFFORTS: readonly ReasoningEffort[] = ['low', 'medium', 'high', 'xhigh', 'max'];

export function claudeModelsFromSdk(models: ModelInfo[]): ClaudeModelOption[] {
  const result: ClaudeModelOption[] = [];
  for (const model of models) {
    const efforts = Array.isArray(model.supportedEffortLevels)
      ? model.supportedEffortLevels.filter((effort) => EFFORTS.includes(effort))
      : [];
    for (const id of [model.value, model.resolvedModel]) {
      if (typeof id !== 'string' || !MODEL_ID.test(id) || result.some(item => item.id === id)) continue;
      result.push({
        id,
        label: id === model.value && isString(model.displayName, 120) ? model.displayName : id,
        reasoningEfforts: efforts
      });
    }
  }
  return result;
}

async function discoverClaudeModels(binary: string, home: string): Promise<ClaudeModelOption[]> {
  // Streaming input lets the SDK answer its control request without sending a user prompt.
  const session = query({
    prompt: (async function* () {})(),
    options: {
      pathToClaudeCodeExecutable: binary,
      env: isolatedClaudeEnvironment(home),
      tools: [],
      settingSources: []
    }
  });
  let timer: NodeJS.Timeout | null = null;
  try {
    const models = await Promise.race([
      session.supportedModels(),
      new Promise<never>((_, reject) => {
        timer = setTimeout(() => reject(new Error('Claude model discovery timed out.')), MODEL_TIMEOUT_MS);
      })
    ]);
    return claudeModelsFromSdk(models);
  } finally {
    if (timer) clearTimeout(timer);
    session.close();
  }
}

type SpawnLike = (command: string, args: readonly string[], options: SpawnOptions) => ChildProcess;

interface ClaudeAccountDependencies {
  discoverBinary(preference: string | null): Promise<string>;
  prepareHome(userData: string): Promise<string>;
  spawnProcess: SpawnLike;
  statusTimeoutMs: number;
  loginTimeoutMs: number;
}

const disconnected = (detail: string | null = null): ClaudeAccountStatus => ({
  state: 'disconnected', email: null, planType: null, detail
});

const unavailable = (detail: string): ClaudeAccountStatus => ({
  state: 'unavailable', email: null, planType: null, detail: detail.slice(0, MAX_DETAIL_CHARS)
});

function errorMessage(error: unknown): string {
  if (error instanceof Error && error.message.trim()) return error.message.trim();
  return String(error || 'Claude Code is unavailable.').slice(0, MAX_DETAIL_CHARS);
}

export function claudeAccountStatusFromJson(value: unknown): ClaudeAccountStatus {
  if (!isRecord(value) || value.loggedIn !== true) return disconnected();
  return {
    state: 'connected',
    email: isString(value.email, 320) ? value.email : null,
    planType: isString(value.subscriptionType, 80)
      ? value.subscriptionType
      : (isString(value.authMethod, 80) ? value.authMethod : null),
    detail: null
  };
}

export class ClaudeAccountClient {
  private readonly deps: ClaudeAccountDependencies;
  private readonly listeners = new Set<(status: ClaudeAccountStatus) => void>();
  private current: ClaudeAccountStatus = {
    state: 'checking', email: null, planType: null, detail: null
  };
  private statusRequest: Promise<ClaudeAccountStatus> | null = null;
  private modelsRequest: Promise<ClaudeModelOption[]> | null = null;
  private loginChild: ChildProcess | null = null;
  private loginTimer: NodeJS.Timeout | null = null;

  constructor(
    private readonly options: { userData: string; claudeBinaryPref(): string | null },
    dependencies: Partial<ClaudeAccountDependencies> = {}
  ) {
    this.deps = {
      discoverBinary: discoverClaudeBinary,
      prepareHome: prepareIsolatedClaudeHome,
      spawnProcess: spawnClaudeProcess,
      statusTimeoutMs: STATUS_TIMEOUT_MS,
      loginTimeoutMs: LOGIN_TIMEOUT_MS,
      ...dependencies
    };
  }

  onChanged(listener: (status: ClaudeAccountStatus) => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  status(): Promise<ClaudeAccountStatus> {
    if (this.current.state === 'connecting') return Promise.resolve(this.current);
    if (this.statusRequest !== null) return this.statusRequest;
    this.statusRequest = this.readStatus().finally(() => { this.statusRequest = null; });
    return this.statusRequest;
  }

  models(): Promise<ClaudeModelOption[]> {
    if (this.modelsRequest !== null) return this.modelsRequest;
    this.modelsRequest = (async () => {
      const [binary, home] = await Promise.all([
        this.deps.discoverBinary(this.options.claudeBinaryPref()),
        this.deps.prepareHome(this.options.userData)
      ]);
      return discoverClaudeModels(binary, home);
    })().finally(() => { this.modelsRequest = null; });
    return this.modelsRequest;
  }

  private async readStatus(): Promise<ClaudeAccountStatus> {
    try {
      const result = await this.command(['auth', 'status'], this.deps.statusTimeoutMs, true);
      const text = result.stdout.trim() || result.stderr.trim();
      let parsed: unknown;
      try { parsed = JSON.parse(text); } catch { parsed = null; }
      if (parsed !== null) return this.publish(claudeAccountStatusFromJson(parsed));
      if (result.code === 126 || result.code === 127) return this.publish(unavailable('Claude could not be started. Check its installation and try again.'));
      if (result.code !== 0) return this.publish(disconnected('Sign in to Claude to use your subscription.'));
      return this.publish(disconnected());
    } catch (error) {
      return this.publish(unavailable(errorMessage(error)));
    }
  }

  async connect(): Promise<ClaudeAccountStatus> {
    if (this.current.state === 'connecting' && this.loginChild !== null) return this.current;
    const existing = await this.status();
    if (existing.state === 'connected' || existing.state === 'connecting' || existing.state === 'unavailable') {
      return existing;
    }

    try {
      const [binary, home] = await Promise.all([
        this.deps.discoverBinary(this.options.claudeBinaryPref()),
        this.deps.prepareHome(this.options.userData)
      ]);
      const child = this.deps.spawnProcess(binary, ['auth', 'login', '--claudeai'], {
        env: isolatedClaudeEnvironment(home),
        stdio: ['ignore', 'pipe', 'pipe'],
        windowsHide: true
      });
      this.loginChild = child;
      let diagnostic = '';
      child.stdout?.on('data', (chunk: Buffer) => { diagnostic = `${diagnostic}${chunk.toString('utf8')}`.slice(-4_000); });
      child.stderr?.on('data', (chunk: Buffer) => { diagnostic = `${diagnostic}${chunk.toString('utf8')}`.slice(-4_000); });
      child.once('error', (error) => this.finishLogin(error));
      child.once('close', (code) => {
        this.finishLogin(code === 0 ? null : new Error(diagnostic.trim() || 'Claude sign-in was not completed.'));
      });
      this.loginTimer = setTimeout(() => {
        if (this.loginChild !== child) return;
        child.kill('SIGTERM');
        this.finishLogin(new Error('Claude sign-in timed out. Try connecting again.'));
      }, this.deps.loginTimeoutMs);
      return this.publish({
        state: 'connecting', email: null, planType: null,
        detail: 'Finish signing in in the browser opened by Claude Code.'
      });
    } catch (error) {
      return this.publish(unavailable(errorMessage(error)));
    }
  }

  async disconnect(): Promise<ClaudeAccountStatus> {
    this.stopLogin();
    try {
      const result = await this.command(['auth', 'logout'], this.deps.statusTimeoutMs, false);
      if (result.code !== 0) throw new Error(result.stderr.trim() || 'Claude Code could not sign out.');
      return this.publish(disconnected());
    } catch (error) {
      return this.publish(unavailable(errorMessage(error)));
    }
  }

  async shutdown(): Promise<void> {
    this.stopLogin();
  }

  private publish(status: ClaudeAccountStatus): ClaudeAccountStatus {
    this.current = status;
    for (const listener of this.listeners) listener(status);
    return status;
  }

  private finishLogin(error: Error | null): void {
    if (this.loginChild === null) return;
    this.stopLogin(false);
    if (error) {
      this.publish(disconnected(error.message.slice(0, MAX_DETAIL_CHARS)));
      return;
    }
    this.current = { state: 'checking', email: null, planType: null, detail: null };
    void this.status();
  }

  private stopLogin(kill = true): void {
    const child = this.loginChild;
    this.loginChild = null;
    if (this.loginTimer) clearTimeout(this.loginTimer);
    this.loginTimer = null;
    if (kill && child && !child.killed) child.kill('SIGTERM');
  }

  private async command(
    args: readonly string[],
    timeoutMs: number,
    allowNonZero: boolean
  ): Promise<{ code: number | null; stdout: string; stderr: string }> {
    const [binary, home] = await Promise.all([
      this.deps.discoverBinary(this.options.claudeBinaryPref()),
      this.deps.prepareHome(this.options.userData)
    ]);
    const child = this.deps.spawnProcess(binary, args, {
      env: isolatedClaudeEnvironment(home),
      stdio: ['ignore', 'pipe', 'pipe'],
      windowsHide: true
    });
    const stdout: Buffer[] = [];
    const stderr: Buffer[] = [];
    child.stdout?.on('data', (chunk: Buffer) => stdout.push(chunk));
    child.stderr?.on('data', (chunk: Buffer) => stderr.push(chunk));
    return await new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        child.kill('SIGTERM');
        reject(new Error('Claude Code took too long to respond.'));
      }, timeoutMs);
      child.once('error', (error) => { clearTimeout(timer); reject(error); });
      child.once('close', (code) => {
        clearTimeout(timer);
        const result = {
          code,
          stdout: Buffer.concat(stdout).toString('utf8'),
          stderr: Buffer.concat(stderr).toString('utf8')
        };
        if (!allowNonZero && code !== 0) reject(new Error(result.stderr.trim() || `Claude Code exited with ${code}.`));
        else resolve(result);
      });
    });
  }
}
