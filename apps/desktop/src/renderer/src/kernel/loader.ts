/*
 * Extension loader: records → load order → activate → health.
 *
 * Recovery is the whole point of this file. Nothing an extension does may
 * prevent boot, so every stage is fenced:
 *   - import and activate run in try/catch with a timeout;
 *   - a failure disposes the partial registration and records `activation-error`;
 *   - two guarded-callback throws inside 10 s auto-disable the extension.
 * The kernel keeps running with one fewer contributor and the Mods panel shows
 * why.
 */
import type { Disposable, ExtensionModule, UIAPI } from './api';
import type { ExtensionHealth, ExtensionManifest, ExtensionRecord, ExtensionsBridge, ExtensionsChangedEvent } from '../../../shared/extensions';
import { MANIFEST_LIMITS } from '../../../shared/extensions';
import { createExtensionAPI, type ExtensionHandle, type HostDeps } from './host';

/* The loader speaks about an extension rather than as one, so it stamps the
   attribution itself. `source` is host-side and not part of the extension's
   own toast options. */
type ToastArgs = Parameters<UIAPI['toast']>[1];
import type { Kernel } from './registries';
import { activeIds as storeActiveIds, patchRecord, recordFor, records as storeRecords, setActiveIds, setHealth, setRecords } from './extensions.svelte';

export type BuiltinFactory = () => Promise<ExtensionModule>;

export interface LoaderOptions {
  kernel: Kernel;
  bridge: ExtensionsBridge | null;
  /** id → dynamic import of the built-in module. Key order IS the load order. */
  builtins: Record<string, BuiltinFactory>;
  deps: Omit<HostDeps, 'reportRuntimeError'>;
  /** Manifest overrides for synthesized built-in records (tests / bridge-less boot). */
  builtinManifests?: Record<string, Partial<ExtensionManifest>>;
  activateTimeoutMs?: number;
  runtimeErrorWindowMs?: number;
  runtimeErrorLimit?: number;
}

export interface LoadPlan {
  order: string[];
  skipped: Array<{ id: string; health: ExtensionHealth }>;
}

export interface Loader {
  /** Resolves after bundled extensions have activated, before bridge extensions start. */
  readonly builtinsReady: Promise<void>;
  boot(): Promise<void>;
  activate(record: ExtensionRecord): Promise<boolean>;
  deactivate(id: string): Promise<void>;
  reload(id: string): Promise<void>;
  reportRuntimeError(id: string, error: unknown): void;
  records(): ExtensionRecord[];
  activeIds(): string[];
  /** Resolves once every queued bridge-driven reload has settled (test seam). */
  whenIdle(): Promise<void>;
  dispose(): Promise<void>;
}

const DEFAULT_TIMEOUT = 10_000;
const DEFAULT_WINDOW = 10_000;
const DEFAULT_LIMIT = 2;

/** Health states that mean "there is nothing worth trying to import". */
const BLOCKED = new Set(['build-error', 'manifest-error', 'needs-update']);

const nameOf = (record: ExtensionRecord | undefined, id: string): string => record?.manifest?.name ?? id;

export function errorText(error: unknown): string {
  const raw = error instanceof Error ? error.message || String(error) : String(error);
  return raw.length > MANIFEST_LIMITS.errorChars ? `${raw.slice(0, MANIFEST_LIMITS.errorChars - 1)}…` : raw;
}

/* ── load order ──────────────────────────────────────────── */

/**
 * Pure ordering pass: built-ins in `builtinOrder`, then user/project extensions
 * topologically by `dependsOn` with an alphabetical tie-break so boot order is
 * deterministic. Anything unloadable comes back in `skipped` with the health to
 * record for it.
 */
export function planLoad(records: ExtensionRecord[], builtinOrder: string[]): LoadPlan {
  const skipped: Array<{ id: string; health: ExtensionHealth }> = [];
  const byId = new Map(records.map((record) => [record.id, record]));
  const candidates = new Set<string>();

  for (const record of records) {
    if (record.enabled === false) {
      skipped.push({ id: record.id, health: { state: 'disabled' } });
      continue;
    }
    if (BLOCKED.has(record.health?.state)) continue; // main already recorded why
    candidates.add(record.id);
  }

  // `replaces` wins over the replaced extension for as long as it is enabled.
  const replacedBy = new Map<string, string>();
  for (const id of candidates) {
    for (const target of byId.get(id)?.manifest?.replaces ?? []) {
      if (target !== id && candidates.has(target) && !replacedBy.has(target)) replacedBy.set(target, id);
    }
  }
  for (const [target, by] of replacedBy) {
    candidates.delete(target);
    skipped.push({ id: target, health: { state: 'replaced', by } });
  }

  // Drop anything whose dependencies are not going to load (cascading).
  for (let changed = true; changed; ) {
    changed = false;
    for (const id of [...candidates]) {
      for (const dep of byId.get(id)?.manifest?.dependsOn ?? []) {
        if (candidates.has(dep)) continue;
        candidates.delete(id);
        skipped.push({ id, health: { state: 'activation-error', error: `requires ${dep}` } });
        changed = true;
        break;
      }
    }
  }

  const builtins = builtinOrder.filter((id) => candidates.has(id) && byId.get(id)?.scope === 'builtin');
  const builtinSet = new Set(builtins);
  const rest = [...candidates].filter((id) => !builtinSet.has(id)).sort();
  const pending = new Set(rest);
  const order = [...builtins];

  while (pending.size) {
    const ready = [...pending]
      .filter((id) => (byId.get(id)?.manifest?.dependsOn ?? []).every((dep) => !pending.has(dep)))
      .sort();
    const next = ready[0];
    if (!next) break; // everything left is in a cycle
    pending.delete(next);
    order.push(next);
  }
  for (const id of pending) skipped.push({ id, health: { state: 'activation-error', error: 'dependency cycle' } });

  return { order, skipped };
}

/* ── loader ──────────────────────────────────────────────── */

interface ActiveEntry {
  record: ExtensionRecord;
  module: ExtensionModule;
  handle: ExtensionHandle;
}

export function createLoader(options: LoaderOptions): Loader {
  const { kernel, bridge, builtins, deps } = options;
  const timeoutMs = options.activateTimeoutMs ?? DEFAULT_TIMEOUT;
  const windowMs = options.runtimeErrorWindowMs ?? DEFAULT_WINDOW;
  const limit = options.runtimeErrorLimit ?? DEFAULT_LIMIT;

  const active = new Map<string, ActiveEntry>();
  const failures = new Map<string, number[]>();
  const activationFailures = new Set<string>();
  const reloadTokens = new Map<string, number>();
  let queue: Promise<void> = Promise.resolve();
  let booting: Promise<void> | null = null;
  let unsubscribe: (() => void) | null = null;
  let disposed = false;
  let resolveBuiltinsReady!: () => void;
  const builtinsReady = new Promise<void>((resolve) => void (resolveBuiltinsReady = resolve));

  const hostDeps: HostDeps = { ...deps, reportRuntimeError: (id, error) => reportRuntimeError(id, error) };

  const syncActive = (): void => setActiveIds([...active.keys()]);

  function reportHealth(id: string, health: ExtensionHealth): void {
    setHealth(id, health);
    if (health.state === 'ok' || health.state === 'activation-error' || health.state === 'runtime-error') {
      try {
        bridge?.reportHealth({ id, health });
      } catch (error) {
        console.error('[kernel] reportHealth failed', error);
      }
    }
  }

  function synthesizeRecords(): ExtensionRecord[] {
    return Object.keys(builtins).map((id) => ({
      id,
      scope: 'builtin' as const,
      manifest: { id, name: id, version: '1.0.0', apiVersion: 1, ...(options.builtinManifests?.[id] ?? {}) },
      dir: `builtin:${id}`,
      enabled: true,
      bundleUrl: null,
      bundleHash: null,
      health: { state: 'ok' as const },
      updatedAt: 0
    }));
  }

  async function refreshRecords(): Promise<ExtensionRecord[]> {
    let next: ExtensionRecord[];
    if (bridge) {
      try {
        next = await bridge.list();
      } catch (error) {
        console.error('[kernel] extensions bridge list() failed', error);
        next = storeRecords();
      }
    } else {
      next = synthesizeRecords();
    }
    setRecords(next);
    return next;
  }

  function importModule(record: ExtensionRecord): Promise<ExtensionModule> {
    const factory = builtins[record.id];
    if (factory) return Promise.resolve(factory());
    const url = record.bundleUrl;
    if (!url) return Promise.reject(new Error('no bundle to load'));
    return import(/* @vite-ignore */ cacheBust(url, reloadTokens.get(record.id) ?? 0)) as Promise<ExtensionModule>;
  }

  async function activate(record: ExtensionRecord): Promise<boolean> {
    if (disposed || active.has(record.id)) return active.has(record.id);
    const id = record.id;

    let module: ExtensionModule;
    try {
      module = await withTimeout(importModule(record), timeoutMs, `import of "${id}" timed out`);
      if (!module || typeof module.default !== 'function') throw new Error('entry module must export default activate(api)');
    } catch (error) {
      failActivation(record, error);
      return false;
    }

    const handle = createExtensionAPI(kernel, record, hostDeps);
    try {
      const result = await withTimeout(Promise.resolve(module.default(handle.api)), timeoutMs, `activate() of "${id}" timed out`);
      const disposable = result as Disposable | void;
      if (disposable && typeof disposable.dispose === 'function') handle.api.onDispose(() => disposable.dispose());
    } catch (error) {
      handle.disposeAll();
      failActivation(record, error);
      return false;
    }

    active.set(id, { record, module, handle });
    activationFailures.delete(id);
    syncActive();
    reportHealth(id, { state: 'ok' });
    kernel.events.emit('extension:loaded', { id });
    return true;
  }

  function failActivation(record: ExtensionRecord, error: unknown): void {
    const id = record.id;
    activationFailures.add(id);
    console.error(`[kernel] extension "${id}" failed to activate`, error);
    kernel.disposeOwner(id);
    reportHealth(id, { state: 'activation-error', error: errorText(error) });
    const name = nameOf(recordFor(id) ?? record, id);
    /* An extension that will not load is that extension's alert. The editor is
       still whole, and the notice belongs to whoever broke. */
    deps.ui.toast(`${name} failed to load`, { sticky: true, source: { id, name } } as ToastArgs);
  }

  async function deactivate(id: string): Promise<void> {
    const entry = active.get(id);
    active.delete(id);
    syncActive();
    if (entry) {
      try {
        await entry.module.deactivate?.();
      } catch (error) {
        console.error(`[kernel] extension "${id}" deactivate() threw`, error);
      }
      entry.handle.disposeAll();
    }
    kernel.disposeOwner(id);
    failures.delete(id);
    if (entry) kernel.events.emit('extension:unloaded', { id });
  }

  /** Bring the whole active graph back to the current load plan. A toggle can
      affect more than its own id: replacements expose their built-in fallback,
      and dependencies can enable or disable a chain of extensions. Keep panel
      registry changes batched so a replacement swap never presents the layout
      with a momentary "panel missing" state. */
  async function reconcile(list: ExtensionRecord[], changedIds: string[] = []): Promise<void> {
    for (const id of changedIds) activationFailures.delete(id);
    // Health notifications echo our own activation result. Retrying from that
    // notification floods IPC, logs and toasts indefinitely. A failed module
    // stays out of the plan until its source changes or the user retries it;
    // excluding it also lets a replaced built-in become available again.
    const plan = planLoad(list.filter(record => !activationFailures.has(record.id)), Object.keys(builtins));
    const desired = new Set(plan.order);
    const changed = new Set(changedIds);
    const panelChanges = kernel.panels.batchChanges();
    try {
      for (const { id, health } of plan.skipped) setHealth(id, health);
      for (const id of [...active.keys()].reverse()) {
        if (!desired.has(id) || changed.has(id)) await deactivate(id);
      }
      for (const id of plan.order) {
        if (active.has(id)) continue;
        const record = recordFor(id);
        if (record) await activate(record);
      }
    } finally {
      panelChanges.dispose();
    }
  }

  async function reloadInternal(id: string): Promise<void> {
    const panelChanges = kernel.panels.batchChanges();
    try {
      await deactivate(id);
      activationFailures.delete(id);
      reloadTokens.set(id, (reloadTokens.get(id) ?? 0) + 1);
      await refreshRecords();
      const record = recordFor(id);
      if (!record || record.enabled === false || BLOCKED.has(record.health?.state)) return;
      await activate(record);
    } finally { panelChanges.dispose(); }
  }

  function reportRuntimeError(id: string, error: unknown): void {
    const now = Date.now();
    const stamps = (failures.get(id) ?? []).filter((t) => now - t < windowMs);
    stamps.push(now);
    failures.set(id, stamps);
    if (stamps.length < limit) return;
    failures.delete(id);
    const name = nameOf(recordFor(id), id);
    enqueue(async () => {
      await deactivate(id);
      patchRecord(id, { enabled: false });
      reportHealth(id, { state: 'runtime-error', error: errorText(error) });
      deps.ui.toast(`${name} stopped working — check Mods`, { sticky: true, source: { id, name } } as ToastArgs);
    });
  }

  function enqueue(task: () => Promise<void>): Promise<void> {
    const pending = queue.then(task);
    queue = pending.catch((error) => void console.error('[kernel] loader task failed', error));
    return pending;
  }

  // Health reports can echo back while an extension import is still pending.
  // Startup and explicit reloads must share the bridge-change queue; otherwise
  // two activations can both pass active.has(), leaving an untracked runtime
  // (and its canvas painters, timers and input handlers) alive indefinitely.
  function boot(): Promise<void> {
    return booting ??= enqueue(bootInternal);
  }

  function reload(id: string): Promise<void> {
    return enqueue(() => reloadInternal(id));
  }

  async function bootInternal(): Promise<void> {
    const builtinOrder = Object.keys(builtins);
    const builtinIds = new Set(builtinOrder);
    const bundled = synthesizeRecords();
    setRecords(bundled);

    // Bundled extensions are the editor's baseline UI and behavior. Activate
    // them without waiting for the filesystem-backed extension bridge.
    const builtinPlan = planLoad(bundled, builtinOrder);
    for (const { id, health } of builtinPlan.skipped) setHealth(id, health);
    for (const id of builtinPlan.order) {
      const record = recordFor(id);
      if (record) await activate(record);
    }
    kernel.events.emit('extensions:changed', { ids: builtinOrder, reason: 'boot' });
    resolveBuiltinsReady();

    if (!bridge) return;

    const builtinHealth = new Map(builtinOrder.map((id) => [id, recordFor(id)?.health] as const));
    const list = await refreshRecords();
    for (const [id, health] of builtinHealth) if (health) setHealth(id, health);
    const plan = planLoad(list, builtinOrder);
    for (const { id, health } of plan.skipped) {
      if (active.has(id)) await deactivate(id);
      setHealth(id, health);
    }
    for (const id of plan.order) {
      const record = recordFor(id);
      if (record && !builtinIds.has(id) && record.scope !== 'builtin') await activate(record);
    }
  }

  function onChanged(event: ExtensionsChangedEvent): void {
    kernel.events.emit('extensions:changed', { ids: [...event.ids], reason: event.reason });
    enqueue(async () => {
      const list = await refreshRecords();
      /* Health events are round-trips from this renderer. Re-plan so a failed
         replacement can expose its fallback, but do not reload the reporting
         extension and start another report loop. */
      await reconcile(list, event.reason === 'health' ? [] : event.ids);
    });
  }

  if (bridge) {
    try {
      unsubscribe = bridge.onChanged(onChanged);
    } catch (error) {
      console.error('[kernel] extensions bridge onChanged() failed', error);
    }
  }

  return {
    builtinsReady,
    boot,
    activate,
    deactivate,
    reload,
    reportRuntimeError,
    records: () => storeRecords(),
    activeIds: () => storeActiveIds(),
    whenIdle: () => queue,
    async dispose() {
      disposed = true;
      unsubscribe?.();
      unsubscribe = null;
      await queue;
      for (const id of [...active.keys()]) await deactivate(id);
    }
  };
}

/* ── helpers ─────────────────────────────────────────────── */

export function cacheBust(url: string, token: number): string {
  if (!token) return url;
  return `${url}${url.includes('?') ? '&' : '?'}r=${token}`;
}

export function withTimeout<T>(promise: Promise<T>, ms: number, message: string): Promise<T> {
  if (!(ms > 0)) return promise;
  return new Promise<T>((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error(message)), ms);
    promise.then(
      (value) => {
        clearTimeout(timer);
        resolve(value);
      },
      (error) => {
        clearTimeout(timer);
        reject(error);
      }
    );
  });
}
