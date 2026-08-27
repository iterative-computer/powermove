// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { resetExtensionsStore } from './extensions.svelte';
import { bootExtensions, installKernel, type InstalledKernel } from './install';
import { RUNTIME_GLOBAL, runtimeGlobals } from './runtime-globals';
import { doc } from '../state/document.svelte';
import { sel } from '../state/selection.svelte';
import { perf, transport } from '../state/transport.svelte';

type LegacyPM = Record<string, any>;

function fakePM(): LegacyPM {
  const listeners = new Map<string, Set<(...args: any[]) => void>>();
  const store: Record<string, unknown> = {};
  const workspace = { id: 'design', panels: ['viewer'] };
  const PM: LegacyPM = {
    proj: { id: 'p', revision: 7 },
    sel: { layers: ['L1'], keys: ['k1', 4], chan: 'position.x' },
    time: 12,
    playing: false,
    PANELS: { viewer: {} },
    bus: {
      on: (event: string, fn: (...args: any[]) => void) => {
        const set = listeners.get(event) ?? new Set();
        set.add(fn);
        listeners.set(event, set);
        return () => void set.delete(fn);
      },
      emit: (event: string, ...args: any[]) => listeners.get(event)?.forEach((fn) => fn(...args))
    },
    Edit: { apply: vi.fn(() => ({ ok: true, message: 'ok', data: {} })) },
    hist: { undo: vi.fn(), redo: vi.fn() },
    Export: { snapshot: vi.fn(async () => 'data:image/jpeg;base64,x') },
    selectLayers: vi.fn(),
    setTime: vi.fn(),
    play: vi.fn(),
    pause: vi.fn(),
    toast: vi.fn(),
    menu: vi.fn(),
    icon: (name: string) => `<svg data-icon="${name}"></svg>`,
    modal: vi.fn((opts: any) => ({ close: vi.fn(), body: document.createElement('div'), opts })),
    palette: vi.fn(),
    cmd: vi.fn(),
    store: {
      get: (key: string, fallback: unknown) => store[key] ?? fallback,
      set: (key: string, value: unknown) => void (store[key] = value)
    },
    WS: { current: workspace, mutate: vi.fn((fn: (w: unknown) => void) => fn(workspace)) },
    Layout: { addPanel: vi.fn(), removePanel: vi.fn(), hasPanel: vi.fn(() => false), refresh: vi.fn() },
    listeners,
    store_: store
  };
  return PM;
}

let installed: InstalledKernel | null = null;

beforeEach(() => resetExtensionsStore());

afterEach(() => {
  installed?.uninstall();
  installed = null;
  delete (globalThis as Record<string, any>)[RUNTIME_GLOBAL];
  delete (globalThis as Record<string, any>).powermove;
});

describe('installKernel', () => {
  it('installs the svelte runtime globals and publishes the kernel on PM', () => {
    const PM = fakePM();
    installed = installKernel(PM);

    expect(PM.Kernel).toBe(installed);
    expect(runtimeGlobals()?.svelte.mount).toBeTypeOf('function');
    expect(runtimeGlobals()?.['svelte/store'].writable).toBeTypeOf('function');
    expect(installed.loader).toBeNull();
  });

  it('builds a project façade over PM and forces the extension origin', async () => {
    const PM = fakePM();
    installed = installKernel(PM);
    const api = installed.api('demo');

    expect(api.project.get()).toBe(PM.proj);
    expect(api.project.revision()).toBe(7);
    expect(api.project.time()).toBe(12);
    expect(api.project.playing()).toBe(false);
    expect(api.project.selection()).toEqual({ layers: ['L1'], keys: ['k1'], chan: 'position.x' });

    api.project.apply({ type: 'set_layer', target: 'L1', patch: {} } as never, { label: 'Nudge' } as never);
    expect(PM.Edit.apply).toHaveBeenCalledWith({ type: 'set_layer', target: 'L1', patch: {} }, { label: 'Nudge', origin: 'ext:demo' });

    api.project.select(['L2'], true);
    api.project.setTime(3);
    api.project.play();
    api.project.pause();
    api.project.undo();
    api.project.redo();
    expect(PM.selectLayers).toHaveBeenCalledWith(['L2'], true);
    expect(PM.setTime).toHaveBeenCalledWith(3);
    expect(PM.hist.undo).toHaveBeenCalled();
    expect(await api.project.snapshot()).toBe('data:image/jpeg;base64,x');
  });

  it('namespaces storage under ext.<id> as one JSON object', () => {
    const PM = fakePM();
    installed = installKernel(PM);
    const api = installed.api('notes');

    api.storage.set('draft', 'hello');
    api.storage.set('count', 2);
    expect(PM.store.get('ext.notes', {})).toEqual({ draft: 'hello', count: 2 });

    api.storage.delete('draft');
    expect(PM.store.get('ext.notes', {})).toEqual({ count: 2 });
    expect(api.storage.get('count')).toBe(2);
    expect(installed.api('other').storage.get('count')).toBeUndefined();
  });

  it('routes ui helpers to the overlay members on PM', async () => {
    const PM = fakePM();
    installed = installKernel(PM);
    const api = installed.api('demo');

    api.ui.toast('hi');
    expect(PM.toast).toHaveBeenCalledWith('hi', 2200, {});
    api.ui.toast('sticky', { sticky: true });
    expect(PM.toast).toHaveBeenLastCalledWith('sticky', 8000, { sticky: true });

    api.ui.menu({ x: 10, y: 20 }, [{ label: 'A' }]);
    expect(PM.menu).toHaveBeenCalledWith(document.body, [{ label: 'A' }], { x: 10, y: 20 });

    const confirmed = api.ui.confirm('Delete?', 'Cannot undo');
    const options = PM.modal.mock.calls[0][0];
    options.actions[1].run();
    await expect(confirmed).resolves.toBe(true);
    expect(api.ui.icon('play')).toContain('data-icon="play"');
    expect(Object.keys(api.ui.controls)).toEqual([
      'NumField', 'ColorField', 'FillField', 'FontField', 'SelectField',
      'TextField', 'ToggleField', 'Row', 'Section', 'binding'
    ]);
    expect(api.ui.controls.binding).toEqual(expect.objectContaining({
      channelBinding: expect.any(Function),
      layerFieldBinding: expect.any(Function),
      contentBinding: expect.any(Function),
      compositionBinding: expect.any(Function)
    }));
    expect(api.host.state).toEqual({ doc, sel, transport, perf });
  });

  it('opens and closes panels through Layout + WS', () => {
    const PM = fakePM();
    installed = installKernel(PM);
    const api = installed.api('demo');
    api.panels.register({ id: 'notes', title: 'Notes', build: () => {} });

    api.panels.open('notes', 'right');
    expect(PM.WS.mutate).toHaveBeenCalled();
    expect(PM.Layout.addPanel).toHaveBeenCalledWith(PM.WS.current, 'notes', 'right');
    expect(PM.Layout.refresh).toHaveBeenCalledWith('notes');

    api.panels.close('notes');
    expect(PM.Layout.removePanel).toHaveBeenCalledWith(PM.WS.current, 'notes');
    expect(api.panels.list()).toEqual(expect.arrayContaining(['notes', 'viewer']));
  });

  it('bridges the legacy bus onto kernel events and unsubscribes on uninstall', () => {
    const PM = fakePM();
    installed = installKernel(PM);
    const seen: string[] = [];
    installed.events.on('project:changed', (payload) => void seen.push(`project:${payload.kind}`));
    installed.events.on('selection', (payload) => void seen.push(`sel:${payload.layers.join(',')}`));
    installed.events.on('time', (t) => void seen.push(`time:${t}`));
    installed.events.on('transport', (payload) => void seen.push(`play:${payload.playing}`));
    installed.events.on('layout', () => void seen.push('layout'));

    PM.bus.emit('layers');
    PM.bus.emit('assets');
    PM.bus.emit('sel');
    PM.bus.emit('time', 5);
    PM.playing = true;
    PM.bus.emit('transport');
    PM.bus.emit('layout');

    expect(seen).toEqual(['project:structure', 'project:assets', 'sel:L1', 'time:5', 'play:true', 'layout']);

    installed.uninstall();
    installed = null;
    PM.bus.emit('layers');
    expect(seen).toHaveLength(6);
  });

  it('routes key chords through the PM.cmd seam when present', () => {
    const PM = fakePM();
    installed = installKernel(PM);
    const run = vi.fn();
    installed.api('demo').commands.register({ id: 'demo.go', label: 'Go', run });
    installed.api('demo').keybindings.bind({ key: 'cmd+shift+g', command: 'demo.go' });

    const event = new KeyboardEvent('keydown', { key: 'g', metaKey: true, shiftKey: true, bubbles: true, cancelable: true });
    window.dispatchEvent(event);

    /* PM.cmd is the app's public command seam (native menu IPC and tests
       interpose on it); the listener must go through it, not around it. */
    expect(PM.cmd).toHaveBeenCalledWith('demo.go');
    expect(event.defaultPrevented).toBe(true);
  });

  it('falls back to the kernel command table when PM.cmd is absent', () => {
    const PM = fakePM();
    delete (PM as Record<string, unknown>).cmd;
    installed = installKernel(PM);
    const run = vi.fn();
    installed.api('demo').commands.register({ id: 'demo.go', label: 'Go', run });
    installed.api('demo').keybindings.bind({ key: 'cmd+shift+g', command: 'demo.go' });

    const event = new KeyboardEvent('keydown', { key: 'g', metaKey: true, shiftKey: true, bubbles: true, cancelable: true });
    window.dispatchEvent(event);

    expect(run).toHaveBeenCalledTimes(1);
    expect(event.defaultPrevented).toBe(true);
  });

  it('opens the palette through whichever PM shape exists', () => {
    const PM = fakePM();
    installed = installKernel(PM);
    installed.api('demo').palette.open('fx');
    expect(PM.palette).toHaveBeenCalledWith('fx');
  });

  it('tolerates an empty PM without throwing', () => {
    const PM: LegacyPM = {};
    installed = installKernel(PM);
    const api = installed.api('demo');

    expect(() => api.ui.toast('hi')).not.toThrow();
    expect(api.project.selection()).toEqual({ layers: [], keys: [], chan: null });
    expect(api.project.revision()).toBe(0);
    expect(api.project.apply([] as never)).toEqual({ ok: false, message: 'editing engine unavailable' });
    expect(api.storage.get('anything')).toBeUndefined();
    expect(() => api.panels.open('x')).not.toThrow();
    expect(api.ui.icon('play')).toBe('');
    expect(installed.bridge).toBeNull();
  });

  it('bootExtensions creates the loader and activates the given built-ins', async () => {
    const PM = fakePM();
    installed = installKernel(PM);
    const log: string[] = [];
    const loader = await bootExtensions(installed, {
      'effects-basic': async () => ({
        default: (api) => {
          log.push('effects-basic');
          api.effects.register({ id: 'wave', label: 'Wave', group: 'Distort', params: [], frag: 'o = src(v_st);' });
        }
      })
    });

    expect(log).toEqual(['effects-basic']);
    expect(installed.loader).toBe(loader);
    expect(installed.effects.get('wave')?.label).toBe('Wave');
    expect(loader.activeIds()).toEqual(['effects-basic']);
    expect(installed.deps.extensions.list().map((record) => record.id)).toEqual(['effects-basic']);

    await loader.dispose();
    expect(installed.effects.list()).toEqual([]);
  });

  it('emits layout after built-ins without waiting for a hanging user extension', async () => {
    vi.useFakeTimers();
    const PM = fakePM();
    const emit = vi.spyOn(PM.bus, 'emit');
    PM.Layout.ws = PM.WS.current;
    PM.Layout.apply = vi.fn(() => PM.bus.emit('layout'));
    const slowModule = encodeURIComponent('export default function(){ return new Promise(() => {}) }');
    PM.extensionsBridge = {
      list: vi.fn(async () => [
        {
          id: 'toolbar',
          scope: 'builtin',
          manifest: { id: 'toolbar', name: 'Toolbar', version: '1.0.0', apiVersion: 1 },
          dir: 'builtin:toolbar',
          enabled: true,
          bundleUrl: null,
          bundleHash: null,
          health: { state: 'ok' },
          updatedAt: 0
        },
        {
          id: 'slow-user',
          scope: 'user',
          manifest: { id: 'slow-user', name: 'Slow user', version: '1.0.0', apiVersion: 1 },
          dir: '/ext/slow-user',
          enabled: true,
          bundleUrl: `data:text/javascript,${slowModule}`,
          bundleHash: 'slow',
          health: { state: 'ok' },
          updatedAt: 0
        }
      ]),
      setEnabled: vi.fn(),
      remove: vi.fn(),
      reload: vi.fn(),
      create: vi.fn(),
      reveal: vi.fn(),
      readSource: vi.fn(),
      reportHealth: vi.fn(),
      onChanged: vi.fn(() => () => {})
    };
    installed = installKernel(PM);

    let settled = false;
    const booting = bootExtensions(installed, {
      toolbar: async () => ({ default: (api) => void api.panels.register({ id: 'toolbar', title: 'Toolbar', build: () => {} }) })
    }).then((loader) => {
      settled = true;
      return loader;
    });
    await vi.advanceTimersByTimeAsync(0);

    expect(emit).toHaveBeenCalledWith('layout');
    expect(PM.Layout.apply).toHaveBeenCalledWith(PM.WS.current);
    expect(settled).toBe(false);

    await vi.advanceTimersByTimeAsync(10_000);
    await booting;
    expect(emit.mock.calls.filter((call) => call[0] === 'layout')).toHaveLength(2);
    expect(PM.Layout.apply).toHaveBeenCalledTimes(2);
  });
});
