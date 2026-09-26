/*
 * The publish-time sandbox check (sandbox design §7).
 *
 * Runs a folder's compiled bundle the way the loader runs a Store install:
 * the same runtime iframe and the same main-set CSP from the manifest's
 * permissions (sandbox-host.ts), then docks every panel it registers once in
 * a hidden view iframe (sandbox-view.ts). It watches for up to `waitMs`, then
 * tears everything down and returns what it saw.
 *
 * Nothing reaches the app. Registrations land in a scratch kernel, and the
 * host services the sandbox calls are quiet copies: project reads are real,
 * edits and transport changes are dropped, storage writes stay in memory,
 * toasts, pickers and panel opens do nothing.
 *
 * Wiring: renderer-initiated. The publish sheet and the Library's "Test in
 * Sandbox…" call this directly in the window that shows the result; main is
 * not involved beyond serving the sandbox document for the local folder
 * (main/extensions `sandboxManifestFor`). Publishing is gated in the sheet.
 */
import type { ExtensionPermission, ExtensionRecord } from '../../../shared/extensions';
import type { EditResult, ProjectAPI, StorageAPI } from './api';
import type { HostDeps } from './host';
import { panelFrameOf } from './panel-frame';
import { createKernel, type Kernel } from './registries';
import { createSandboxRuntime, type SandboxObserver, type SandboxRuntime, type SandboxRuntimeOptions } from './sandbox-host';

export interface SandboxPermissionHit { namespace: string; member: string; count: number }
export interface SandboxCspHit { directive: string; blockedUri: string }
export interface SandboxAsyncHit { member: string; count: number }
export interface SandboxPanelResult { id: string; mounted: boolean; error?: string }

export interface SandboxCheckReport {
  ok: boolean;
  /** Set when the manifest declares `full-access`: it never runs sandboxed, so nothing ran. */
  skipped?: 'full-access';
  activation: 'ok' | { error: string };
  /** Trusted-only members reached (they throw `PermissionError('full-access')`). */
  permissionErrors: SandboxPermissionHit[];
  cspViolations: SandboxCspHit[];
  /** apiVersion ≤ 2 code that read a sync result from a method that returns a Promise in the sandbox. */
  asyncMisuse: SandboxAsyncHit[];
  panels: SandboxPanelResult[];
  /** Anything else that threw while it ran. */
  runtimeErrors: string[];
  durationMs: number;
}

export interface SandboxCheckOptions {
  /** The app's kernel: theme, key table and the catalog the sandbox reads. Never registered into. */
  kernel: Kernel;
  deps: Omit<HostDeps, 'reportRuntimeError'>;
  /** Defaults to the manifest's. Main serves the document only for the manifest's own list. */
  permissions?: readonly ExtensionPermission[];
  vars?: Record<string, string>;
  /** How long activation plus panel mounting may take. */
  waitMs?: number;
  /** After everything settled: how long to keep listening for late errors. */
  settleMs?: number;
  /** Test seams, passed through to the runtime (see sandbox-host.test.ts). */
  runtime?: Pick<SandboxRuntimeOptions, 'frame' | 'onPostInit' | 'onViewInit'>;
}

const DEFAULT_WAIT_MS = 5_000;
const DEFAULT_SETTLE_MS = 300;
const noop = (): void => {};

function emptyReport(): SandboxCheckReport {
  return { ok: false, activation: 'ok', permissionErrors: [], cspViolations: [], asyncMisuse: [], panels: [], runtimeErrors: [], durationMs: 0 };
}

const messageOf = (error: unknown): string => (error instanceof Error ? error.message || error.name : String(error));

/** Host services for a check: reads are real, everything that would change the app is dropped. */
export function quietDeps(deps: Omit<HostDeps, 'reportRuntimeError'>, onError: (error: unknown) => void): HostDeps {
  const real = deps.project;
  const project: ProjectAPI = {
    get: () => real.get(), revision: () => real.revision(), selection: () => real.selection(),
    time: () => real.time(), playing: () => real.playing(),
    apply: (): EditResult => ({ ok: true, message: 'Not applied during a sandbox check', data: {} }),
    select: noop, setTime: noop, play: noop, pause: noop, undo: noop, redo: noop,
    snapshot: async () => ''
  };
  const storage = (id: string): StorageAPI => {
    const backing = deps.storage(id);
    const written = new Map<string, unknown>();
    const removed = new Set<string>();
    return {
      get: <T,>(key: string) => (written.has(key) ? written.get(key) : removed.has(key) ? undefined : backing.get(key)) as T | undefined,
      set: (key, value) => { written.set(key, value); removed.delete(key); },
      delete: (key) => { written.delete(key); removed.add(key); }
    };
  };
  return {
    ...deps,
    project,
    transport: deps.transport ? { ...deps.transport, setTime: noop, play: noop, pause: noop, toggle: noop, step: noop, invalidate: noop } : undefined,
    ui: { ...deps.ui, toast: noop, confirm: async () => false, menu: noop },
    assets: {
      pick: async () => [],
      import: async () => { throw new Error('Importing is off during a sandbox check'); },
      get: (id) => deps.assets.get(id),
      readText: (id) => deps.assets.readText(id)
    },
    storage,
    extensions: {
      list: () => deps.extensions.list(),
      setEnabled: async () => {}, remove: async () => {}, reload: async () => {}, reveal: async () => {},
      requestFix: noop, rebase: noop
    },
    panelsBackend: { open: noop, close: noop, refresh: noop, isOpen: () => false, isHidden: () => false, list: () => deps.panelsBackend.list() },
    paletteOpen: noop,
    reportRuntimeError: (_id, error) => onError(error)
  };
}

function withDeadline<T>(promise: Promise<T>, ms: number, message: string): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  return Promise.race([
    promise,
    new Promise<never>((_resolve, reject) => { timer = setTimeout(() => reject(new Error(message)), ms); })
  ]).finally(() => clearTimeout(timer));
}

const seconds = (ms: number): string => {
  const value = Math.round(ms / 100) / 10;
  return `${value} ${value === 1 ? 'second' : 'seconds'}`;
};

/** `render.gl` → `render` + `gl`; `powermove.resolveContent` → `powermove` + `resolveContent`. */
function splitMember(path: string): { namespace: string; member: string } {
  const dot = path.indexOf('.');
  return dot < 0 ? { namespace: path, member: '' } : { namespace: path.slice(0, dot), member: path.slice(dot + 1) };
}

/** Activate `record`'s compiled bundle in a hidden sandbox, mount its panels once, and report. */
export async function runSandboxCheck(record: ExtensionRecord, options: SandboxCheckOptions): Promise<SandboxCheckReport> {
  const started = performance.now();
  const waitMs = options.waitMs ?? DEFAULT_WAIT_MS;
  const report = emptyReport();
  const finish = (): SandboxCheckReport => {
    report.durationMs = Math.round(performance.now() - started);
    report.ok = report.skipped !== undefined || (
      report.activation === 'ok' && !report.permissionErrors.length && !report.cspViolations.length &&
      !report.asyncMisuse.length && !report.runtimeErrors.length && report.panels.every((panel) => panel.mounted)
    );
    return report;
  };

  const permissions = [...(options.permissions ?? record.manifest?.permissions ?? [])];
  if (permissions.includes('full-access')) {
    report.skipped = 'full-access';
    return finish();
  }
  if (!record.manifest || !record.bundleUrl) {
    report.activation = { error: record.health?.state === 'build-error' ? `It doesn’t build: ${record.health.error}` : 'It has no compiled bundle yet. Save it once so Powermove builds it, then check again.' };
    return finish();
  }

  const permissionCounts = new Map<string, number>();
  const asyncCounts = new Map<string, number>();
  const csp = new Map<string, SandboxCspHit>();
  const errors = new Set<string>();
  const panelStates = new Map<string, { state: 'ready' | 'error'; message?: string }>();
  const panelWaiters = new Map<string, () => void>();
  const observer: SandboxObserver = {
    permission: (member) => void permissionCounts.set(member, (permissionCounts.get(member) ?? 0) + 1),
    asyncMisuse: (member) => void asyncCounts.set(member, (asyncCounts.get(member) ?? 0) + 1),
    csp: (directive, blockedUri) => void csp.set(`${directive} ${blockedUri}`, { directive, blockedUri }),
    view: (panelId, state, message) => {
      if (panelStates.has(panelId)) return; // the first outcome of the one mount counts
      panelStates.set(panelId, { state, ...(message ? { message } : {}) });
      panelWaiters.get(panelId)?.();
    }
  };

  const registry = createKernel();
  const deps = quietDeps(options.deps, (error) => errors.add(messageOf(error)));
  const target: ExtensionRecord = { ...record, trust: 'store', manifest: { ...record.manifest, permissions } };
  const stage = document.createElement('div');
  stage.setAttribute('aria-hidden', 'true');
  stage.inert = true;
  stage.style.cssText = 'position:fixed;left:-10000px;top:0;width:360px;height:480px;overflow:hidden;opacity:0;pointer-events:none;contain:strict';
  let runtime: SandboxRuntime | null = null;
  const starting = createSandboxRuntime(options.kernel, target, deps, options.vars ?? {}, { ...options.runtime, registry, observer });

  try {
    try {
      runtime = await withDeadline(starting, waitMs, `It didn’t finish activating within ${seconds(waitMs)}.`);
    } catch (error) {
      report.activation = { error: messageOf(error) };
      // A runtime that still arrives after the deadline is torn down at once.
      void starting.then((late) => late.dispose(), noop);
    }

    if (runtime) {
      document.body.append(stage);
      const frames = registry.panels.list().filter((def) => panelFrameOf(def)?.extensionId === record.id);
      const outcomes = frames.map((def) => new Promise<void>((resolve) => {
        if (panelStates.has(def.id)) { resolve(); return; }
        panelWaiters.set(def.id, resolve);
        const body = document.createElement('div');
        body.style.cssText = 'position:absolute;inset:0';
        stage.append(body);
        try { def.build?.(body, { spec: {} }); }
        catch (error) { observer.view?.(def.id, 'error', messageOf(error)); }
      }));
      const remaining = Math.max(0, waitMs - (performance.now() - started));
      await withDeadline(Promise.all(outcomes), remaining, 'Panel mounting timed out').catch(noop);
      for (const def of frames) {
        const state = panelStates.get(def.id);
        report.panels.push(state?.state === 'ready'
          ? { id: def.id, mounted: true }
          : { id: def.id, mounted: false, error: state?.message ?? `It didn’t finish loading within ${seconds(waitMs)}.` });
      }
      const settle = options.settleMs ?? DEFAULT_SETTLE_MS;
      if (settle > 0) await new Promise((resolve) => setTimeout(resolve, settle));
    }
  } finally {
    try { runtime?.dispose(); } catch (error) { errors.add(messageOf(error)); }
    stage.remove();
    registry.dispose();
  }

  report.permissionErrors = [...permissionCounts].map(([path, count]) => ({ ...splitMember(path), count }));
  report.asyncMisuse = [...asyncCounts].map(([member, count]) => ({ member, count }));
  report.cspViolations = [...csp.values()];
  /* A PermissionError that aborted activation is already a permission hit. */
  const activationError = report.activation === 'ok' ? null : report.activation.error;
  report.runtimeErrors = [...errors].filter((message) => message !== activationError);
  return finish();
}
