/*
 * Updates for the remote host. The npm registry is the source of truth: the
 * release workflow publishes `powermove-cli` at the same version as the Mac
 * app. The host asks the registry on start, on a timer, and when a tab asks,
 * and reports the same AppUpdateState the desktop's updater does, so the web
 * client's existing update UI works unchanged.
 *
 * Installing depends on how the host got here. A global npm install can
 * replace itself and restart (a service brings it back; a terminal run is
 * told to restart). An `npx` run cannot, so the tab is handed the command.
 */
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';

import type { AppUpdateState } from '../shared/ipc';

const run = promisify(execFile);

export const PACKAGE = 'powermove-cli';
export type InstallKind = 'global' | 'npx' | 'source' | 'unknown';

/** Where the running entry lives says how it was installed. */
export function detectInstallKind(entry: string): InstallKind {
  const path = entry.replace(/\\/g, '/');
  if (/\/_npx\//.test(path) || /\/\.npm\/_npx\//.test(path)) return 'npx';
  if (/\/node_modules\/powermove-cli\//.test(path)) return 'global';
  if (/\/packages\/cli\//.test(path)) return 'source';
  return 'unknown';
}

/** semver-ish compare: 1.2.10 > 1.2.9; prereleases sort below their release. */
export function isNewer(candidate: string, current: string): boolean {
  const parse = (v: string) => {
    const [core = '', pre] = v.split('-', 2);
    return { parts: core.split('.').map((n) => Number(n) || 0), pre: pre ?? null };
  };
  const a = parse(candidate), b = parse(current);
  for (let i = 0; i < 3; i++) {
    const x = a.parts[i] ?? 0, y = b.parts[i] ?? 0;
    if (x !== y) return x > y;
  }
  if (a.pre === b.pre) return false;
  if (a.pre === null) return true;
  if (b.pre === null) return false;
  return a.pre > b.pre;
}

export function updateCommand(kind: InstallKind): string {
  switch (kind) {
    case 'global': return `npm i -g ${PACKAGE}@latest`;
    case 'npx': return `npx ${PACKAGE}@latest serve`;
    case 'source': return 'git pull && bun run --cwd packages/cli build';
    default: return `npm i -g ${PACKAGE}@latest`;
  }
}

export interface UpdateCheckerOptions {
  current: string;
  installKind: InstallKind;
  /** Stable hosts follow `latest`; a beta host follows `beta`. */
  channel?: 'latest' | 'beta';
  fetchLatest?: (channel: string) => Promise<string | null>;
  intervalMs?: number;
  log?: (line: string) => void;
  /** Runs the self-update; resolves when the new version is in place. */
  selfUpdate?: () => Promise<void>;
  /** Ends this process so the service (or the user) restarts it. */
  exit?: () => void;
}

async function registryLatest(channel: string): Promise<string | null> {
  const response = await fetch(`https://registry.npmjs.org/${PACKAGE}/${channel}`, { signal: AbortSignal.timeout(10_000), headers: { accept: 'application/json' } });
  if (!response.ok) return null;
  const data = (await response.json()) as { version?: unknown };
  return typeof data.version === 'string' ? data.version : null;
}

export class UpdateChecker {
  private state: AppUpdateState;
  private readonly listeners = new Set<(state: AppUpdateState) => void>();
  private timer: ReturnType<typeof setInterval> | null = null;
  private checking: Promise<void> | null = null;

  constructor(private readonly options: UpdateCheckerOptions) {
    this.state = { status: 'idle', current: options.current, version: null };
  }

  status(): AppUpdateState { return this.state; }
  onChanged(listener: (state: AppUpdateState) => void): () => void {
    this.listeners.add(listener);
    return () => { this.listeners.delete(listener); };
  }

  start(): void {
    void this.check();
    this.timer = setInterval(() => { void this.check(); }, this.options.intervalMs ?? 6 * 60 * 60 * 1000);
    this.timer.unref?.();
  }
  stop(): void { if (this.timer) clearInterval(this.timer); this.timer = null; }

  check(): Promise<void> {
    if (this.checking) return this.checking;
    this.checking = (async () => {
      if (this.state.status === 'ready') return;
      this.set({ status: 'checking', current: this.options.current, version: null });
      try {
        const latest = await (this.options.fetchLatest ?? registryLatest)(this.options.channel ?? 'latest');
        if (latest && isNewer(latest, this.options.current)) this.set({ status: 'ready', current: this.options.current, version: latest });
        else this.set({ status: 'idle', current: this.options.current, version: null });
      } catch (error) {
        this.options.log?.(`[updates] check failed: ${error instanceof Error ? error.message : String(error)}`);
        this.set({ status: 'idle', current: this.options.current, version: null });
      }
    })().finally(() => { this.checking = null; });
    return this.checking;
  }

  /** Self-update when possible; otherwise the command the user runs. */
  async install(): Promise<{ restarting: true } | { restarting: false; command: string }> {
    const command = updateCommand(this.options.installKind);
    if (this.options.installKind !== 'global') return { restarting: false, command };
    if (this.state.status !== 'ready') await this.check();
    if (this.state.status !== 'ready') return { restarting: false, command };
    this.set({ ...this.state, status: 'downloading' });
    try {
      await (this.options.selfUpdate ?? (async () => { await run('npm', ['i', '-g', `${PACKAGE}@${this.state.version}`], { timeout: 10 * 60 * 1000 }); }))();
    } catch (error) {
      this.options.log?.(`[updates] self-update failed: ${error instanceof Error ? error.message : String(error)}`);
      this.set({ ...this.state, status: 'ready' });
      return { restarting: false, command };
    }
    this.options.log?.(`[updates] installed ${this.state.version}; restarting`);
    setTimeout(() => (this.options.exit ?? (() => process.exit(0)))(), 300).unref?.();
    return { restarting: true };
  }

  private set(next: AppUpdateState): void {
    this.state = next;
    for (const listener of this.listeners) listener(next);
  }
}
