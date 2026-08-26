// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ExtensionModule, ExtensionRecord, Project, ProjectAPI } from './api';
import type { ExtensionsBridge, ExtensionsChangedEvent, ExtensionHealthReport } from '../../../shared/extensions';
import { resetExtensionsStore } from './extensions.svelte';
import type { HostDeps } from './host';
import { createLoader, planLoad, type BuiltinFactory } from './loader';
import { createKernel, type Kernel } from './registries';

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
      ui: {
        toast: (text) => void toasts.push(text),
        confirm: async () => true,
        menu: () => {},
        modal: () => ({ close: () => {}, body: document.createElement('div') }),
        icon: () => ''
      },
      project,
      storage: () => ({ get: () => undefined, set: () => {}, delete: () => {} }),
      extensions: { list: () => [], setEnabled: async () => {}, remove: async () => {}, reload: async () => {}, reveal: async () => {}, requestFix: () => {} },
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
});

/* ── boot / activate ─────────────────────────────────────── */

describe('loader boot', () => {
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
