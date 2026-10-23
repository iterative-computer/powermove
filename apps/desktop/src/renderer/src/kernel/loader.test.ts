// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ExtensionModule, ExtensionRecord, PowermoveAPI, Project, ProjectAPI } from './api';
import type { ExtensionsBridge, ExtensionsChangedEvent, ExtensionHealthReport } from '../../../shared/extensions';
import { resetExtensionsStore } from './extensions.svelte';
import type { HostDeps } from './host';
import { createLoader, planLoad, type BuiltinFactory } from './loader';
import { createKernel, type Kernel } from './registries';
import { createSandboxRuntime } from './sandbox-host';

vi.mock('./sandbox-host', async (original) => {
  const actual = await original<typeof import('./sandbox-host')>();
  return { ...actual, createSandboxRuntime: vi.fn(actual.createSandboxRuntime) };
});

/* ── fixtures ────────────────────────────────────────────── */

function rec(id: string, over: Partial<ExtensionRecord> = {}): ExtensionRecord {
  const { manifest, ...rest } = over;
  return {
    id,
    scope: 'user',
    manifest: { id, name: id, version: '1.0.0', apiVersion: 1, ...(manifest ?? {}) },
    dir: `/ext/${id}`,
    enabled: true,
    bundleUrl: null,
    bundleHash: null,
    health: { state: 'ok' },
    updatedAt: 0,
    ...rest
  };
}

const builtinRec = (id: string, over: Partial<ExtensionRecord> = {}): ExtensionRecord => rec(id, { scope: 'builtin', ...over });

function fakeBridge(records: ExtensionRecord[]) {
  const listeners: Array<(event: ExtensionsChangedEvent) => void> = [];
  const health: ExtensionHealthReport[] = [];
  const state = { records };
  const bridge: ExtensionsBridge = {
    list: async () => state.records.map((record) => ({ ...record })),
    setEnabled: async () => state.records,
    remove: async () => state.records,
    reload: async () => state.records,
    create: async () => state.records,
    reveal: async () => {},
    readSource: async () => [],
    reportHealth: (report) => void health.push(report),
    onChanged: (cb) => {
      listeners.push(cb);
      return () => void listeners.splice(listeners.indexOf(cb), 1);
    }
  };
  return {
    bridge,
    health,
    state,
    emit: (event: ExtensionsChangedEvent) => listeners.forEach((cb) => cb(event))
  };
}

function fakeDeps(): { deps: Omit<HostDeps, 'reportRuntimeError'>; toasts: string[] } {
  const toasts: string[] = [];
  const project = {
    get: () => ({}) as Project,
    revision: () => 0,
    apply: () => ({ ok: true as const, message: 'ok', data: {} }),
    selection: () => ({ layers: [], keys: [], chan: null }),
    select: () => {},
    time: () => 0,
    setTime: () => {},
    play: () => {},
    pause: () => {},
    playing: () => false,
    undo: () => {},
    redo: () => {},
    snapshot: async () => ''
  } as unknown as ProjectAPI;
  return {
    toasts,
    deps: {
      pm: {},
      state: { doc: {}, sel: {}, transport: {}, perf: {} },
      ui: {
        controls: {} as HostDeps['ui']['controls'],
        toast: (text) => void toasts.push(text),
        confirm: async () => true,
        openExternal: async () => true,
        menu: () => {},
        modal: () => ({ close: () => {}, body: document.createElement('div') }),
        icon: () => ''
      },
      project,
      assets: { pick: async () => [], import: async (file) => ({ id: 'a', name: file.name, kind: 'model' }), get: () => undefined, readText: async () => '', importUrl: async () => 'a' },
      storage: () => ({ get: () => undefined, set: () => {}, delete: () => {} }),
      extensions: { list: () => [], setEnabled: async () => {}, remove: async () => {}, reload: async () => {}, reveal: async () => {}, requestFix: () => {}, rebase: () => {} },
      panelsBackend: { open: () => {}, close: () => {}, isOpen: () => false, refresh: () => {}, list: () => [] },
      paletteOpen: () => {}
    }
  };
}

/** A built-in factory that records activation order into `log`. */
function mod(log: string[], id: string, extra: Partial<ExtensionModule> = {}): BuiltinFactory {
  return async () => ({
    default: (api) => {
      log.push(id);
      api.commands.register({ id: `${id}.hello`, label: id, run: () => id });
    },
    ...extra
  });
}

let kernel: Kernel;
let errorSpy: ReturnType<typeof vi.spyOn>;

beforeEach(() => {
  resetExtensionsStore();
  kernel = createKernel();
  errorSpy = vi.spyOn(console, 'error').mockImplementation(() => {});
});

afterEach(() => {
  errorSpy.mockRestore();
  vi.useRealTimers();
});

/* ── planLoad ────────────────────────────────────────────── */

describe('planLoad', () => {
  it('loads built-ins in key order, then user extensions topologically with an alphabetical tie-break', () => {
    const plan = planLoad(
      [
        rec('zulu'),
        rec('alpha', { manifest: { id: 'alpha', name: 'a', version: '1.0.0', apiVersion: 1, dependsOn: ['zulu'] } }),
        builtinRec('theme-default'),
        builtinRec('effects-basic')
      ],
      ['effects-basic', 'theme-default']
    );
    expect(plan.order).toEqual(['effects-basic', 'theme-default', 'zulu', 'alpha']);
    expect(plan.skipped).toEqual([]);
  });

  it('skips disabled extensions', () => {
    const plan = planLoad([rec('a'), rec('b', { enabled: false })], []);
    expect(plan.order).toEqual(['a']);
    expect(plan.skipped).toEqual([{ id: 'b', health: { state: 'disabled' } }]);
  });

  it('deactivates a replaced extension and records who replaced it', () => {
    const plan = planLoad(
      [builtinRec('timeline'), rec('timeline-pro', { manifest: { id: 'timeline-pro', name: 'p', version: '1.0.0', apiVersion: 1, replaces: ['timeline'] } })],
      ['timeline']
    );
    expect(plan.order).toEqual(['timeline-pro']);
    expect(plan.skipped).toEqual([{ id: 'timeline', health: { state: 'replaced', by: 'timeline-pro' } }]);
  });

  it('reports missing dependencies and cascades to their dependents', () => {
    const plan = planLoad(
      [
        rec('a', { manifest: { id: 'a', name: 'a', version: '1.0.0', apiVersion: 1, dependsOn: ['missing'] } }),
        rec('b', { manifest: { id: 'b', name: 'b', version: '1.0.0', apiVersion: 1, dependsOn: ['a'] } })
      ],
      []
    );
    expect(plan.order).toEqual([]);
    expect(plan.skipped).toContainEqual({ id: 'a', health: { state: 'activation-error', error: 'requires missing' } });
    expect(plan.skipped).toContainEqual({ id: 'b', health: { state: 'activation-error', error: 'requires a' } });
  });

  it('breaks dependency cycles with an activation error', () => {
    const plan = planLoad(
      [
        rec('a', { manifest: { id: 'a', name: 'a', version: '1.0.0', apiVersion: 1, dependsOn: ['b'] } }),
        rec('b', { manifest: { id: 'b', name: 'b', version: '1.0.0', apiVersion: 1, dependsOn: ['a'] } })
      ],
      []
    );
    expect(plan.order).toEqual([]);
    expect(plan.skipped).toEqual([
      { id: 'a', health: { state: 'activation-error', error: 'dependency cycle' } },
      { id: 'b', health: { state: 'activation-error', error: 'dependency cycle' } }
    ]);
  });

  it('never plans an extension main already marked build- or manifest-broken', () => {
    const plan = planLoad([rec('a', { health: { state: 'build-error', error: 'esbuild' } }), rec('b')], []);
    expect(plan.order).toEqual(['b']);
  });

  it('never plans untrusted store code that asks for full access', () => {
    const full = { manifest: { id: 'x', name: 'x', version: '1.0.0', apiVersion: 3, permissions: ['full-access' as const] } };
    const plan = planLoad([
      // Main withholds the bundle and says so...
      rec('held', { ...full, trust: 'store', health: { state: 'needs-trust' } }),
      // ...and a record that claims ok is still refused.
      rec('claims-ok', { ...full, trust: 'store', bundleUrl: 'data:text/javascript,export default () => {}' }),
      rec('trusted', { ...full, trust: 'store-trusted' }),
      rec('local', { ...full, trust: 'local' }),
      rec('plain', { trust: 'store' })
    ], []);
    expect(plan.order).toEqual(['local', 'plain', 'trusted']);
    expect(plan.skipped).toContainEqual({ id: 'claims-ok', health: { state: 'needs-trust' } });
  });
});

/* ── boot / activate ─────────────────────────────────────── */

describe('loader boot', () => {
  it('waits for the mirror before importing or activating an extension, without blocking the caller', async () => {
    const { deps } = fakeDeps();
    let release!: (ok: boolean) => void;
    const doc = { enabled: true, ready: false, whenReady: new Promise<boolean>(r => { release = r; }) };
    (deps.pm as any).EngineDoc = doc;
    const imported = vi.fn(async () => ({ default: vi.fn(() => { expect(doc.ready).toBe(true); }) }));
    const loader = createLoader({ kernel, bridge: null, deps, builtins: { early: imported } });
    const booting = loader.boot();
    await new Promise(r => setTimeout(r, 0));
    expect(imported).not.toHaveBeenCalled();
    expect(loader.activeIds()).toEqual([]);
    doc.ready = true;
    release(true);
    await booting;
    expect(loader.activeIds()).toEqual(['early']);
    expect(imported).toHaveBeenCalledOnce();
    await loader.dispose();
  });

  it('keeps the named module failure out of extension code, and activates what waited once a retry succeeds', async () => {
    const { deps, toasts } = fakeDeps();
    let ready: (() => void) | null = null;
    const doc = { enabled: true, ready: false, whenReady: Promise.resolve(false), onReady: (fn: () => void) => { ready = fn; return () => { ready = null; }; } };
    (deps.pm as any).EngineDoc = doc;
    const order: string[] = [];
    const early = vi.fn(async () => ({ default: () => { expect(doc.ready).toBe(true); order.push('early'); } }));
    const late = vi.fn(async () => ({ default: () => { order.push('late'); } }));
    const loader = createLoader({ kernel, bridge: null, deps, builtins: { early, late } });
    await loader.boot();
    expect(early).not.toHaveBeenCalled();
    expect(loader.activeIds()).toEqual([]);
    expect(loader.records()[0]?.health).toMatchObject({ state: 'activation-error', error: expect.stringContaining('pm-wasm-unavailable') });
    // The engine names the failure once; extensions add no toast of their own.
    expect(toasts).toEqual([]);
    // EngineDoc.retry() succeeded.
    doc.ready = true;
    ready!();
    await loader.whenIdle();
    expect(order).toEqual(['early', 'late']);
    expect(loader.activeIds()).toEqual(['early', 'late']);
    expect(loader.records()[0]?.health).toEqual({ state: 'ok' });
    expect(ready).toBeNull();
    await loader.dispose();
  });

  it('does not activate a second timeline when health notifications arrive during boot', async () => {
    const bridge = fakeBridge([builtinRec('theme'), builtinRec('timeline')]);
    const { deps } = fakeDeps();
    const paints: number[] = [];
    const timelines: Array<{ pps: number }> = [];
    let releaseImport!: (module: ExtensionModule) => void;
    const imported = new Promise<ExtensionModule>(resolve => { releaseImport = resolve; });
    bridge.bridge.reportHealth = report => {
      bridge.emit({ ids: [report.id], reason: 'health' });
    };
    const loader = createLoader({
      kernel, bridge: bridge.bridge, deps,
      builtins: {
        theme: mod([], 'theme'),
        timeline: () => imported,
      },
    });
    const booting = loader.boot();
    // Let the theme's health echo re-enter the loader while timeline imports.
    await new Promise(resolve => setTimeout(resolve, 0));
    releaseImport({ default(api) {
      const timeline = { pps: 90 };
      timelines.push(timeline);
      api.services.register('timeline', timeline as any);
      api.events.on('invalidate', () => { paints.push(timeline.pps); });
    } });
    await booting;
    await loader.whenIdle();
    const timeline = kernel.services.get<any>('timeline')!;
    timeline.pps = 6;
    kernel.events.emit('invalidate', 'timeline');
    await loader.dispose();

    expect(paints).toEqual([6]);
    expect(timelines).toHaveLength(1);
  });

  it('signals built-in readiness before a slow bridge list resolves', async () => {
    let releaseList!: (records: ExtensionRecord[]) => void;
    const bridge = fakeBridge([]);
    bridge.bridge.list = () => new Promise<ExtensionRecord[]>((resolve) => void (releaseList = resolve));
    const events: Array<{ ids: string[]; reason: string }> = [];
    kernel.events.on('extensions:changed', (event) => void events.push(event));
    const log: string[] = [];
    const { deps } = fakeDeps();
    const loader = createLoader({ kernel, bridge: bridge.bridge, deps, builtins: { toolbar: mod(log, 'toolbar') } });

    const booting = loader.boot();
    await loader.builtinsReady;

    expect(log).toEqual(['toolbar']);
    expect(events).toContainEqual({ ids: ['toolbar'], reason: 'boot' });
    releaseList([builtinRec('toolbar')]);
    await booting;
  });

  it('activates built-ins in key order without a bridge and exposes reactive records', async () => {
    const log: string[] = [];
    const { deps } = fakeDeps();
    const loader = createLoader({ kernel, bridge: null, deps, builtins: { first: mod(log, 'first'), second: mod(log, 'second') } });

    await loader.boot();

    expect(log).toEqual(['first', 'second']);
    expect(loader.activeIds()).toEqual(['first', 'second']);
    expect(loader.records().map((record) => record.id)).toEqual(['first', 'second']);
    expect(kernel.commands.has('first.hello')).toBe(true);
    expect(loader.records().every((record) => record.health.state === 'ok')).toBe(true);
  });

  it('honours synthesized built-in manifests for dependency order', async () => {
    const log: string[] = [];
    const { deps } = fakeDeps();
    const loader = createLoader({
      kernel,
      bridge: null,
      deps,
      builtins: { a: mod(log, 'a'), b: mod(log, 'b') },
      builtinManifests: { b: { replaces: ['a'] } }
    });
    await loader.boot();
    expect(log).toEqual(['b']);
    expect(loader.records().find((record) => record.id === 'a')?.health).toEqual({ state: 'replaced', by: 'b' });
  });

  it('records an activation error, disposes partial registrations, and toasts', async () => {
    const { deps, toasts } = fakeDeps();
    const bridge = fakeBridge([rec('bad', { manifest: { id: 'bad', name: 'Bad mod', version: '1.0.0', apiVersion: 1 } })]);
    const loader = createLoader({
      kernel,
      bridge: bridge.bridge,
      deps,
      builtinManifests: { bad: { name: 'Bad mod' } },
      builtins: {
        bad: async () => ({
          default: (api) => {
            api.commands.register({ id: 'ghost', label: 'Ghost', run: () => 1 });
            throw new Error('activate exploded');
          }
        })
      }
    });

    await loader.boot();

    expect(loader.activeIds()).toEqual([]);
    expect(kernel.commands.has('ghost')).toBe(false);
    expect(loader.records()[0]?.health).toEqual({ state: 'activation-error', error: 'activate exploded' });
    expect(bridge.health).toContainEqual({ id: 'bad', health: { state: 'activation-error', error: 'activate exploded' } });
    expect(toasts).toEqual(['Bad mod failed to load']);
  });

  it('rejects a module without a default activate function', async () => {
    const { deps } = fakeDeps();
    const loader = createLoader({ kernel, bridge: null, deps, builtins: { broken: async () => ({}) as never } });
    await loader.boot();
    expect(loader.records()[0]?.health).toEqual({ state: 'activation-error', error: 'entry module must export default activate(api)' });
  });

  it('times out an activate() that never resolves', async () => {
    vi.useFakeTimers();
    const { deps } = fakeDeps();
    const loader = createLoader({
      kernel,
      bridge: null,
      deps,
      activateTimeoutMs: 50,
      builtins: { slow: async () => ({ default: () => new Promise<void>(() => {}) }) }
    });
    const booting = loader.boot();
    await vi.advanceTimersByTimeAsync(60);
    await booting;
    expect(loader.records()[0]?.health).toMatchObject({ state: 'activation-error', error: 'activate() of "slow" timed out' });
  });

  it('emits extension:loaded and extension:unloaded', async () => {
    const events: string[] = [];
    kernel.events.on('extension:loaded', (payload) => void events.push(`+${payload.id}`));
    kernel.events.on('extension:unloaded', (payload) => void events.push(`-${payload.id}`));
    const { deps } = fakeDeps();
    const loader = createLoader({ kernel, bridge: null, deps, builtins: { one: mod([], 'one') } });

    await loader.boot();
    await loader.deactivate('one');
    await loader.deactivate('one'); // no-op, no second event

    expect(events).toEqual(['+one', '-one']);
  });

  it('calls module.deactivate and releases every registration', async () => {
    const bye = vi.fn();
    const { deps } = fakeDeps();
    const loader = createLoader({ kernel, bridge: null, deps, builtins: { one: mod([], 'one', { deactivate: bye }) } });

    await loader.boot();
    expect(kernel.commands.has('one.hello')).toBe(true);

    await loader.deactivate('one');
    expect(bye).toHaveBeenCalledTimes(1);
    expect(kernel.commands.has('one.hello')).toBe(false);
    expect(loader.activeIds()).toEqual([]);
  });

  it('survives a throwing deactivate()', async () => {
    const { deps } = fakeDeps();
    const loader = createLoader({
      kernel,
      bridge: null,
      deps,
      builtins: { one: mod([], 'one', { deactivate: () => { throw new Error('bye boom'); } }) }
    });
    await loader.boot();
    await expect(loader.deactivate('one')).resolves.toBeUndefined();
    expect(kernel.commands.has('one.hello')).toBe(false);
  });
});

/* ── reload & bridge changes ─────────────────────────────── */

describe('loader reload', () => {
  it('serializes a requested reload with bridge changes so the old painter is fully retired', async () => {
    const bridge = fakeBridge([builtinRec('timeline')]);
    const { deps } = fakeDeps();
    const paints: number[] = [];
    const retired: number[] = [];
    let activations = 0;
    let releaseImport!: () => void;
    const importGate = new Promise<void>(resolve => { releaseImport = resolve; });
    let beginImport!: () => void;
    const importing = new Promise<void>(resolve => { beginImport = resolve; });
    const loader = createLoader({
      kernel, bridge: bridge.bridge, deps,
      builtins: { timeline: async () => {
        if (activations) { beginImport(); await importGate; }
        return { default(api) {
          const instance = ++activations;
          api.events.on('invalidate', () => { paints.push(instance); });
          api.onDispose(() => { retired.push(instance); });
        } };
      } },
    });
    await loader.boot();
    const reloading = loader.reload('timeline');
    await importing;
    bridge.emit({ ids: ['timeline'], reason: 'health' });
    await new Promise(resolve => setTimeout(resolve, 0));
    releaseImport();
    await reloading;
    await loader.whenIdle();
    kernel.events.emit('invalidate', 'timeline');
    await loader.dispose();

    expect(activations).toBe(2);
    expect(paints).toEqual([2]);
    expect(retired).toEqual([1, 2]);
  });

  it('does not retry a failed activation on health echoes, but retries changed source', async () => {
    const { deps, toasts } = fakeDeps();
    const bridge = fakeBridge([builtinRec('bad')]);
    let broken = true;
    const activate = vi.fn(() => { if (broken) throw new Error('Unavailable module'); });
    const loader = createLoader({ kernel, bridge: bridge.bridge, deps, builtins: { bad: async () => ({ default: activate }) } });
    await loader.boot();
    for (let i = 0; i < 3; i++) {
      bridge.emit({ ids: ['bad'], reason: 'health' });
      await loader.whenIdle();
    }
    expect(activate).toHaveBeenCalledTimes(1);
    expect(toasts).toHaveLength(1);
    broken = false;
    bridge.emit({ ids: ['bad'], reason: 'watch' });
    await loader.whenIdle();
    expect(activate).toHaveBeenCalledTimes(2);
    expect(loader.activeIds()).toEqual(['bad']);
    await loader.dispose();
  });

  it('reloads an extension by deactivating and re-activating from a fresh record', async () => {
    const log: string[] = [];
    const { deps } = fakeDeps();
    const bridge = fakeBridge([rec('one')]);
    const loader = createLoader({ kernel, bridge: bridge.bridge, deps, builtins: { one: mod(log, 'one') } });

    await loader.boot();
    await loader.reload('one');

    expect(log).toEqual(['one', 'one']);
    expect(loader.activeIds()).toEqual(['one']);
  });

  it('does not re-activate a reloaded extension that is now disabled', async () => {
    const log: string[] = [];
    const { deps } = fakeDeps();
    const bridge = fakeBridge([rec('one')]);
    const loader = createLoader({ kernel, bridge: bridge.bridge, deps, builtins: { one: mod(log, 'one') } });

    await loader.boot();
    bridge.state.records = [rec('one', { enabled: false })];
    await loader.reload('one');

    expect(log).toEqual(['one']);
    expect(loader.activeIds()).toEqual([]);
  });

  it('reacts to bridge onChanged: reloads healthy ids and deactivates the rest', async () => {
    const log: string[] = [];
    const { deps } = fakeDeps();
    const bridge = fakeBridge([rec('one'), rec('two')]);
    const loader = createLoader({ kernel, bridge: bridge.bridge, deps, builtins: { one: mod(log, 'one'), two: mod(log, 'two') } });

    await loader.boot();
    expect(loader.activeIds()).toEqual(['one', 'two']);

    bridge.state.records = [rec('one'), rec('two', { enabled: false })];
    bridge.emit({ ids: ['one', 'two'], reason: 'watch' });
    await loader.whenIdle();

    expect(log).toEqual(['one', 'two', 'one']);
    expect(loader.activeIds()).toEqual(['one']);
  });

  it('swaps a timeline replacement and its built-in fallback whenever either is toggled', async () => {
    const log: string[] = [];
    const { deps } = fakeDeps();
    const timeline = builtinRec('timeline');
    const replacement = rec('timeline-pro', {
      manifest: { id: 'timeline-pro', name: 'Timeline Pro', version: '1.0.0', apiVersion: 1, replaces: ['timeline'] }
    });
    const bridge = fakeBridge([timeline, replacement]);
    const loader = createLoader({
      kernel,
      bridge: bridge.bridge,
      deps,
      builtins: { timeline: mod(log, 'timeline'), 'timeline-pro': mod(log, 'timeline-pro') }
    });

    await loader.boot();
    expect(loader.activeIds()).toEqual(['timeline-pro']);

    bridge.state.records = [timeline, { ...replacement, enabled: false }];
    bridge.emit({ ids: ['timeline-pro'], reason: 'disable' });
    await loader.whenIdle();
    expect(loader.activeIds()).toEqual(['timeline']);

    bridge.state.records = [timeline, replacement];
    bridge.emit({ ids: ['timeline-pro'], reason: 'enable' });
    await loader.whenIdle();
    expect(loader.activeIds()).toEqual(['timeline-pro']);
  });
});

/* ── runtime error policy ────────────────────────────────── */

describe('runtime error policy', () => {
  it('auto-disables after two failures inside the window and toasts once', async () => {
    const { deps, toasts } = fakeDeps();
    const bridge = fakeBridge([rec('flaky', { manifest: { id: 'flaky', name: 'Flaky mod', version: '1.0.0', apiVersion: 1 } })]);
    const loader = createLoader({
      kernel,
      bridge: bridge.bridge,
      deps,
      runtimeErrorWindowMs: 10_000,
      builtins: {
        flaky: async () => ({
          default: (api) => {
            api.commands.register({ id: 'flaky.run', label: 'Flaky', run: () => { throw new Error('kaboom'); } });
          }
        })
      }
    });

    await loader.boot();
    expect(loader.activeIds()).toEqual(['flaky']);

    kernel.commands.get('flaky.run')?.run();
    await loader.whenIdle();
    expect(loader.activeIds()).toEqual(['flaky']);

    kernel.commands.get('flaky.run')?.run();
    await loader.whenIdle();

    expect(loader.activeIds()).toEqual([]);
    expect(kernel.commands.has('flaky.run')).toBe(false);
    expect(loader.records()[0]).toMatchObject({ enabled: false, health: { state: 'runtime-error', error: 'kaboom' } });
    expect(bridge.health).toContainEqual({ id: 'flaky', health: { state: 'runtime-error', error: 'kaboom' } });
    expect(toasts).toEqual(['Flaky mod stopped working — check Mods']);
  });

  it('forgets failures older than the window', async () => {
    vi.useFakeTimers();
    const { deps, toasts } = fakeDeps();
    const loader = createLoader({
      kernel,
      bridge: null,
      deps,
      runtimeErrorWindowMs: 1_000,
      builtins: {
        flaky: async () => ({
          default: (api) => {
            api.commands.register({ id: 'flaky.run', label: 'Flaky', run: () => { throw new Error('kaboom'); } });
          }
        })
      }
    });

    await loader.boot();
    kernel.commands.get('flaky.run')?.run();
    vi.advanceTimersByTime(5_000);
    kernel.commands.get('flaky.run')?.run();
    await loader.whenIdle();

    expect(loader.activeIds()).toEqual(['flaky']);
    expect(toasts).toEqual([]);
  });
});

describe('loader dispose', () => {
  it('unsubscribes from the bridge and deactivates everything', async () => {
    const log: string[] = [];
    const { deps } = fakeDeps();
    const bridge = fakeBridge([rec('one')]);
    const loader = createLoader({ kernel, bridge: bridge.bridge, deps, builtins: { one: mod(log, 'one') } });

    await loader.boot();
    await loader.dispose();

    expect(loader.activeIds()).toEqual([]);
    bridge.emit({ ids: ['one'], reason: 'watch' });
    await loader.whenIdle();
    expect(log).toEqual(['one']);
  });
});


describe('bridge recovery and bundle CSS ownership', () => {
  it('keeps runtime failures quarantined across a health echo until an explicit retry', async () => {
    const bridge = fakeBridge([rec('flaky')]);
    let activations = 0;
    bridge.bridge.reportHealth = (report) => {
      bridge.state.records[0]!.health = report.health;
      bridge.emit({ ids: [report.id], reason: 'health' });
    };
    const loader = createLoader({ kernel, bridge: bridge.bridge, deps: fakeDeps().deps,
      builtins: { flaky: async () => ({ default: () => { activations++; } }) } });
    await loader.boot();
    await loader.whenIdle();
    loader.reportRuntimeError('flaky', new Error('boom'));
    loader.reportRuntimeError('flaky', new Error('boom'));
    await loader.whenIdle();
    await loader.whenIdle();
    expect(activations).toBe(1);
    expect(loader.activeIds()).toEqual([]);
    bridge.emit({ ids: ['flaky'], reason: 'enable' });
    await loader.whenIdle();
    expect(activations).toBe(2);
    await loader.dispose();
  });

  it('skips store + full-access with needs-trust instead of importing it', async () => {
    const records = [rec('theirs', {
      trust: 'store',
      bundleUrl: 'data:text/javascript,globalThis.__trustLeak = true; export default () => {}',
      manifest: { id: 'theirs', name: 'Theirs', version: '1.0.0', apiVersion: 3, permissions: ['full-access'] }
    })];
    const bridge = fakeBridge(records);
    const loader = createLoader({ kernel, bridge: bridge.bridge, deps: fakeDeps().deps, builtins: {} });
    await loader.boot();
    await loader.whenIdle();
    expect(loader.activeIds()).toEqual([]);
    expect((globalThis as { __trustLeak?: boolean }).__trustLeak).toBeUndefined();
    expect(loader.records().find((record) => record.id === 'theirs')?.health).toEqual({ state: 'needs-trust' });
    await loader.dispose();
  });

  it('loads extensions discovered after the renderer initial list completes', async () => {
    const bridge = fakeBridge([]);
    const loader = createLoader({ kernel, bridge: bridge.bridge, deps: fakeDeps().deps, builtins: {} });
    await loader.boot();
    expect(loader.activeIds()).toEqual([]);
    bridge.state.records = [rec('late', { bundleUrl: 'data:text/javascript,export default () => {}' })];
    bridge.emit({ ids: [], reason: 'reload' });
    await loader.whenIdle();
    expect(loader.activeIds()).toEqual(['late']);
    await loader.dispose();
  });

  it('releases styles on disable, reload and failed activation, and reacquires cached styles', async () => {
    const bridge = fakeBridge([rec('styled')]);
    let fail = false;
    const listeners = new Set<(css: string) => void>();
    const module = { __powermoveAcquireStyles: (listener: (css: string) => void) => {
      module.__powermoveStyles.forEach(listener);
      listeners.add(listener);
      return () => { listeners.delete(listener); };
    }, __powermoveStyles: ['.plain { color: red; }', '.svelte-scoped { color: blue; }'],
      default: () => { if (fail) throw new Error('activation failed'); } };
    const loader = createLoader({ kernel, bridge: bridge.bridge, deps: fakeDeps().deps,
      builtins: { styled: async () => module } });
    const styles = () => [...document.head.querySelectorAll('style[data-powermove-extension="styled"]')];
    await loader.boot();
    expect(styles()).toHaveLength(2);
    const initial = styles();
    await loader.reload('styled');
    expect(styles()).toHaveLength(2);
    expect(initial.every(style => !style.isConnected)).toBe(true);
    bridge.state.records[0]!.enabled = false;
    bridge.emit({ ids: ['styled'], reason: 'disable' });
    await loader.whenIdle();
    expect(styles()).toHaveLength(0);
    expect(listeners.size).toBe(0);
    bridge.state.records[0]!.enabled = true;
    bridge.emit({ ids: ['styled'], reason: 'enable' });
    await loader.whenIdle();
    expect(styles()).toHaveLength(2);
    expect(listeners.size).toBe(1);
    for (const listener of listeners) listener('.lazy { color: green; }');
    expect(styles()).toHaveLength(3);
    fail = true;
    await loader.reload('styled');
    expect(styles()).toHaveLength(0);
    expect(listeners.size).toBe(0);
    await loader.dispose();
  });
});

it('publishes record changes after the async bridge read and runtime state before unload', async () => {
  const bridge = fakeBridge([rec('flaky')]);
  const loader = createLoader({ kernel, bridge: bridge.bridge, deps: fakeDeps().deps,
    builtins: { flaky: async () => ({ default: () => undefined }) } });
  await loader.boot();
  const observed: boolean[] = [];
  kernel.events.on('extensions:changed', () => observed.push(loader.records()[0]!.enabled));
  bridge.state.records[0]!.enabled = false;
  bridge.emit({ ids: ['flaky'], reason: 'health' });
  expect(observed).toEqual([]);
  await loader.whenIdle();
  expect(observed).toEqual([false]);
  bridge.state.records[0]!.enabled = true;
  bridge.emit({ ids: ['flaky'], reason: 'enable' });
  await loader.whenIdle();
  const unloaded: ExtensionRecord[] = [];
  kernel.events.on('extension:unloaded', () => unloaded.push(loader.records()[0]!));
  loader.reportRuntimeError('flaky', new Error('boom'));
  loader.reportRuntimeError('flaky', new Error('boom'));
  await loader.whenIdle();
  expect(unloaded).toMatchObject([{ enabled: false, health: { state: 'runtime-error' } }]);
  await loader.dispose();
});

it('turns an extension off at the first sandbox_fatal error', async () => {
  const bridge = fakeBridge([rec('spinner')]);
  const { deps, toasts } = fakeDeps();
  const loader = createLoader({ kernel, bridge: bridge.bridge, deps, builtins: { spinner: async () => ({ default: () => undefined }) } });
  await loader.boot();
  await loader.whenIdle();
  expect(loader.activeIds()).toEqual(['spinner']);
  loader.reportRuntimeError('spinner', Object.assign(new Error('stopped responding'), { code: 'sandbox_fatal' }));
  await loader.whenIdle();
  expect(loader.activeIds()).toEqual([]);
  expect(loader.records()[0]).toMatchObject({ enabled: false, health: { state: 'runtime-error', error: expect.stringContaining('stopped responding') } });
  expect(toasts).toHaveLength(1);
  await loader.dispose();
});

it('turns off a sandbox that spun while activating, with one report and one toast', async () => {
  const store = rec('spinner', { trust: 'store', bundleUrl: 'app://x/bundle.js', manifest: { id: 'spinner', name: 'Spinner', version: '1.0.0', apiVersion: 3 } });
  const bridge = fakeBridge([store]);
  const { deps, toasts } = fakeDeps();
  vi.mocked(createSandboxRuntime).mockRejectedValueOnce(Object.assign(new Error('stopped responding while activating'), { code: 'sandbox_fatal' }));
  const loader = createLoader({ kernel, bridge: bridge.bridge, deps, builtins: {} });
  await loader.boot();
  await loader.whenIdle();
  expect(loader.activeIds()).toEqual([]);
  expect(loader.records()[0]).toMatchObject({ enabled: false, health: { state: 'runtime-error', error: 'stopped responding while activating' } });
  expect(bridge.health).toEqual([{ id: 'spinner', health: { state: 'runtime-error', error: 'stopped responding while activating' } }]);
  expect(toasts).toEqual(['Spinner stopped working — check Mods']);
  await loader.dispose();
});

it('leaves a sandbox its own activation deadline instead of racing it', async () => {
  const store = rec('slow', { trust: 'store', bundleUrl: 'app://x/bundle.js', manifest: { id: 'slow', name: 'Slow', version: '1.0.0', apiVersion: 3 } });
  const { deps, toasts } = fakeDeps();
  const handle = { disposeAll: vi.fn() } as unknown as Awaited<ReturnType<typeof createSandboxRuntime>>['handle'];
  vi.mocked(createSandboxRuntime).mockImplementationOnce(() => new Promise(resolve => setTimeout(resolve, 60, { handle, dispose: vi.fn() })));
  const loader = createLoader({ kernel, bridge: null, deps, builtins: {}, activateTimeoutMs: 20 });
  expect(await loader.activate(store)).toBe(true);
  expect(vi.mocked(createSandboxRuntime).mock.lastCall?.[4]).toEqual({ timeoutMs: 20 });
  expect(toasts).toEqual([]);
  await loader.dispose();
});

describe('api.vars', () => {
  const declared = { apiVersion: 2, vars: [{ key: 'API_KEY', label: 'API key', secret: true, required: true }, { key: 'REGION', label: 'Region' }] };

  function capture(): { factory: BuiltinFactory; seen: Array<PowermoveAPI['vars']> } {
    const seen: Array<PowermoveAPI['vars']> = [];
    return { seen, factory: async () => ({ default: (api) => void seen.push(api.vars) }) };
  }

  it('delivers values fetched before activate, keeping only declared keys', async () => {
    const { deps } = fakeDeps();
    const { factory, seen } = capture();
    const values = vi.fn(async () => ({ API_KEY: 'sk-test', UNDECLARED: 'nope' }));
    const loader = createLoader({ kernel, bridge: null, vars: { values }, deps, builtins: { weather: factory } });
    expect(await loader.activate(rec('weather', { manifest: declared as never }))).toBe(true);
    expect(values).toHaveBeenCalledWith({ id: 'weather' });
    const vars = seen[0]!;
    expect(vars.get('API_KEY')).toBe('sk-test');
    expect(vars.has('API_KEY')).toBe(true);
    expect(vars.get('REGION')).toBeUndefined();
    expect(vars.has('UNDECLARED')).toBe(false);
    expect(vars.keys()).toEqual(['API_KEY']);
  });

  it('gives an extension without vars an empty API and never asks for values', async () => {
    const { deps } = fakeDeps();
    const { factory, seen } = capture();
    const values = vi.fn(async () => ({ API_KEY: 'sk-test' }));
    const loader = createLoader({ kernel, bridge: null, vars: { values }, deps, builtins: { plain: factory } });
    await loader.activate(rec('plain'));
    expect(values).not.toHaveBeenCalled();
    expect(seen[0]!.keys()).toEqual([]);
    expect(seen[0]!.get('API_KEY')).toBeUndefined();
  });

  it('activates with no values when the host cannot deliver them', async () => {
    const { deps } = fakeDeps();
    const { factory, seen } = capture();
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const values = vi.fn(async () => { throw new Error('Variables are not available in powermove serve yet'); });
    const loader = createLoader({ kernel, bridge: null, vars: { values }, deps, builtins: { weather: factory } });
    expect(await loader.activate(rec('weather', { manifest: declared as never }))).toBe(true);
    expect(seen[0]!.keys()).toEqual([]);
    warn.mockRestore();
  });

  it('never imports an extension waiting for setup', () => {
    const plan = planLoad([rec('weather', { health: { state: 'needs-setup', missing: ['API_KEY'] } })], []);
    expect(plan.order).toEqual([]);
  });
});

describe('bounded extension deactivation', () => {
  it('retires a hanging owner before later reloads and bridge changes', async () => {
    vi.useFakeTimers();
    const { deps } = fakeDeps();
    const disposed = vi.fn();
    const log: string[] = [];
    const bridge = fakeBridge([builtinRec('stuck'), builtinRec('healthy')]);
    const loader = createLoader({
      kernel, bridge: bridge.bridge, deps, activateTimeoutMs: 20,
      builtins: {
        stuck: async () => ({
          default: api => {
            log.push('stuck');
            api.commands.register({ id: 'stuck.run', label: 'Stuck', run: () => 'ok' });
            api.onDispose(disposed);
          },
          deactivate: () => new Promise<void>(() => {})
        }),
        healthy: mod(log, 'healthy')
      }
    });
    await loader.boot();
    const stalled = loader.reload('stuck');
    bridge.emit({ ids: ['healthy'], reason: 'watch' });
    let settled = false;
    void Promise.all([stalled, loader.whenIdle()]).then(() => { settled = true; });
    await vi.advanceTimersByTimeAsync(100);
    expect(settled, 'Hanging cleanup must not block the extension change queue').toBe(true);
    expect(disposed).toHaveBeenCalledTimes(1);
    expect(log).toEqual(['stuck', 'healthy', 'stuck', 'healthy']);
    expect(kernel.commands.get('stuck.run')?.run()).toBe('ok');
    expect(loader.activeIds()).toEqual(['stuck', 'healthy']);
    expect(errorSpy).toHaveBeenCalledWith('[kernel] extension "stuck" deactivate() threw', expect.objectContaining({ message: 'deactivate() of "stuck" timed out' }));
    expect(vi.getTimerCount()).toBe(0);
    const closing = loader.dispose();
    await vi.advanceTimersByTimeAsync(21);
    await closing;
  });

  it('bounds shutdown cleanup and releases every host contribution', async () => {
    vi.useFakeTimers();
    const { deps } = fakeDeps();
    const disposed = vi.fn();
    const loader = createLoader({ kernel, bridge: null, deps, activateTimeoutMs: 20, builtins: {
      stuck: async () => ({ default: api => {
        api.commands.register({ id: 'stuck.run', label: 'Stuck', run: () => true });
        api.panels.register({ id: 'stuck.panel', title: 'Stuck', build: () => {} });
        api.onDispose(disposed);
      }, deactivate: () => new Promise<void>(() => {}) })
    } });
    await loader.boot();
    let settled = false;
    const closing = loader.dispose().then(() => { settled = true; });
    await vi.advanceTimersByTimeAsync(21);
    expect(settled, 'Loader shutdown must complete after its cleanup deadline').toBe(true);
    await closing;
    expect(disposed).toHaveBeenCalledTimes(1);
    expect(kernel.commands.has('stuck.run')).toBe(false);
    expect(kernel.panels.has('stuck.panel')).toBe(false);
    expect(loader.activeIds()).toEqual([]);
    expect(vi.getTimerCount()).toBe(0);
  });

  it.each(['throw', 'reject'] as const)('releases registrations when cleanup fails by %s', async mode => {
    vi.useFakeTimers();
    const { deps } = fakeDeps();
    const disposed = vi.fn();
    const failure = new Error('Cleanup failed');
    const loader = createLoader({ kernel, bridge: null, deps, activateTimeoutMs: 20, builtins: {
      broken: async () => ({ default: api => {
        api.commands.register({ id: 'broken.run', label: 'Broken', run: () => true });
        api.onDispose(disposed);
      }, deactivate: () => {
        if (mode === 'throw') throw failure;
        return Promise.reject(failure);
      } })
    } });
    await loader.boot();
    await loader.deactivate('broken');
    expect(kernel.commands.has('broken.run')).toBe(false);
    expect(disposed).toHaveBeenCalledTimes(1);
    expect(errorSpy).toHaveBeenCalledWith('[kernel] extension "broken" deactivate() threw', failure);
    expect(vi.getTimerCount()).toBe(0);
    await loader.dispose();
  });

  it('clears a pending deadline when asynchronous cleanup finishes promptly', async () => {
    vi.useFakeTimers();
    const { deps } = fakeDeps();
    const dispose = vi.fn();
    const loader = createLoader({ kernel, bridge: null, deps, activateTimeoutMs: 20, builtins: {
      clean: async () => ({ default: api => api.onDispose(dispose), deactivate: async () => {} })
    } });
    await loader.boot();
    await loader.deactivate('clean');
    expect(dispose).toHaveBeenCalledTimes(1);
    expect(errorSpy).not.toHaveBeenCalled();
    expect(vi.getTimerCount()).toBe(0);
    await loader.dispose();
  });

  it('cannot let late cleanup remove the newly activated owner', async () => {
    vi.useFakeTimers();
    const { deps } = fakeDeps();
    let release!: () => void;
    const gate = new Promise<void>(resolve => { release = resolve; });
    let latestApi!: PowermoveAPI;
    let latestRegistration!: { dispose(): void };
    let generation = 0;
    const disposed: number[] = [];
    const loader = createLoader({ kernel, bridge: null, deps, activateTimeoutMs: 20, builtins: {
      late: async () => ({
        default: api => {
          const current = ++generation;
          latestApi = api;
          latestRegistration = api.commands.register({ id: 'late.run', label: 'Late', run: () => current });
          api.onDispose(() => disposed.push(current));
        },
        deactivate: async () => {
          const api = latestApi, registration = latestRegistration;
          await gate;
          registration.dispose();
          // A stale async task using its retired API also cannot overwrite the new owner.
          api.commands.register({ id: 'late.run', label: 'Stale', run: () => -1 });
        }
      })
    } });
    await loader.boot();
    const reload = loader.reload('late');
    let settled = false;
    void reload.then(() => { settled = true; });
    await vi.advanceTimersByTimeAsync(21);
    expect(settled).toBe(true);
    await reload;
    expect(disposed).toEqual([1]);
    expect(kernel.commands.get('late.run')?.run()).toBe(2);
    release();
    await vi.advanceTimersByTimeAsync(0);
    expect(kernel.commands.get('late.run')?.run()).toBe(2);
    expect(loader.activeIds()).toEqual(['late']);
    expect(disposed).toEqual([1]);
    expect(vi.getTimerCount()).toBe(0);
    await loader.dispose();
    expect(disposed).toEqual([1, 2]);
  });
});
