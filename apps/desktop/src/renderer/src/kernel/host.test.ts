// @vitest-environment happy-dom
import { describe, expect, it, vi } from 'vitest';
import type { ExtensionRecord, Project, ProjectAPI, StorageAPI } from './api';
import { createExtensionAPI, type HostDeps } from './host';
import { createKernel, type Kernel } from './registries';

function record(id = 'vhs'): ExtensionRecord {
  return {
    id,
    scope: 'user',
    manifest: { id, name: 'VHS effect', version: '1.0.0', apiVersion: 1 },
    dir: `/ext/${id}`,
    enabled: true,
    bundleUrl: `app://powermove/ext/${id}/bundle.js`,
    bundleHash: 'abc',
    health: { state: 'ok' },
    updatedAt: 1
  };
}

function harness(kernel: Kernel = createKernel()) {
  const applied: Array<{ commands: unknown; meta: unknown }> = [];
  const reported: Array<{ id: string; error: unknown }> = [];
  const toasts: string[] = [];
  const toastCalls: Array<{ text: string; opts: unknown }> = [];
  const opened: string[] = [];
  const bag: Record<string, Record<string, unknown>> = {};

  const project = {
    get: () => ({ id: 'p' }) as unknown as Project,
    revision: () => 3,
    apply: (commands: unknown, meta: unknown) => {
      applied.push({ commands, meta });
      return { ok: true as const, message: 'ok', data: {} };
    },
    selection: () => ({ layers: ['L1'], keys: [], chan: null }),
    select: vi.fn(),
    time: () => 2,
    setTime: vi.fn(),
    play: vi.fn(),
    pause: vi.fn(),
    playing: () => false,
    undo: vi.fn(),
    redo: vi.fn(),
    snapshot: async () => 'data:image/jpeg;base64,'
  } as unknown as ProjectAPI;

  const storage = (id: string): StorageAPI => ({
    get: (key) => (bag[id] ?? {})[key] as never,
    set: (key, value) => void ((bag[id] ??= {})[key] = value),
    delete: (key) => void delete (bag[id] ??= {})[key]
  });

  const deps: HostDeps = {
    pm: { legacy: true },
    state: { doc: {}, sel: {}, transport: {}, perf: {} },
    ui: {
      controls: {} as HostDeps['ui']['controls'],
      toast: (text, opts) => { toasts.push(text); toastCalls.push({ text, opts }); },
      confirm: async () => true,
      menu: vi.fn(),
      modal: () => ({ close: () => {}, body: document.createElement('div') }),
      icon: (name) => `<svg data-icon="${name}"></svg>`
    },
    project,
    assets: {
      pick: async () => [],
      import: async (file) => ({ id: 'asset-1', name: file.name, kind: 'model' }),
      get: () => undefined,
      readText: async () => ''
    },
    storage,
    extensions: { list: () => [], setEnabled: async () => {}, remove: async () => {}, reload: async () => {}, reveal: async () => {}, requestFix: vi.fn(), rebase: vi.fn() },
    panelsBackend: {
      open: (id) => void opened.push(id),
      close: vi.fn(),
      isOpen: () => false,
      refresh: vi.fn(),
      list: () => ['viewer']
    },
    paletteOpen: vi.fn(),
    reportRuntimeError: (id, error) => void reported.push({ id, error })
  };

  return { kernel, deps, applied, reported, toasts, toastCalls, opened, bag };
}

describe('createExtensionAPI', () => {
  it('owns import defaults, copies inputs and restores prior overrides on unload', () => {
    const { kernel, deps } = harness();
    const first = createExtensionAPI(kernel, record('first'), deps);
    const second = createExtensionAPI(kernel, record('second'), deps);
    expect(first.api.media.getImportDefaults()).toBeNull();
    const defaults = { anchor: { x: 0.5, y: 0.5 } };
    const registration = first.api.media.registerImportDefaults(defaults);
    defaults.anchor.x = 0;
    const read = first.api.media.getImportDefaults()!;
    read.anchor.y = 1;
    expect(second.api.media.getImportDefaults()).toEqual({ anchor: { x: 0.5, y: 0.5 } });
    second.api.media.registerImportDefaults({ anchor: { x: 0, y: 1 } });
    expect(first.api.media.getImportDefaults()).toEqual({ anchor: { x: 0, y: 1 } });
    second.disposeAll();
    expect(first.api.media.getImportDefaults()).toEqual({ anchor: { x: 0.5, y: 0.5 } });
    for (const x of [NaN, Infinity, -1, 2, '0.5']) {
      expect(() => first.api.media.registerImportDefaults({ anchor: { x: x as number, y: 0.5 } })).toThrow('finite numbers');
    }
    registration.dispose();
    expect(first.api.media.getImportDefaults()).toBeNull();
  });

  it('owns Properties sections, guards failures and disposes mounted controls once', () => {
    const { kernel, deps, reported } = harness();
    const changed = vi.fn();
    kernel.events.on('inspector:changed', changed);
    const handle = createExtensionAPI(kernel, record(), deps);
    const cleanup = vi.fn();
    handle.api.inspector.registerSection({ id: 'anchors', title: 'Anchor presets', build(target) {
      target.textContent = 'Nine anchor presets';
      return cleanup;
    } });
    const target = document.createElement('div');
    const mounted = handle.api.inspector.sections()[0]!.build(target, { layerIds: ['a'] });
    expect(target.textContent).toBe('Nine anchor presets');
    handle.disposeAll();
    if (mounted && typeof mounted !== 'function') mounted.dispose();
    expect(cleanup).toHaveBeenCalledOnce();
    expect(target.childNodes).toHaveLength(0);
    expect(kernel.inspectorSections.list()).toEqual([]);
    expect(changed).toHaveBeenCalledTimes(2);
    const broken = createExtensionAPI(kernel, record('broken'), deps);
    broken.api.inspector.registerSection({ id: 'broken', title: 'Broken', build() { throw new Error('bad control'); } });
    expect(() => broken.api.inspector.sections()[0]!.build(target, { layerIds: [] })).not.toThrow();
    expect(reported).toEqual([expect.objectContaining({ id: 'broken', error: expect.any(Error) })]);
  });

  it('attributes every notice to the extension that raised it', () => {
    const { kernel, deps, toastCalls } = harness();
    const handle = createExtensionAPI(kernel, record(), deps);

    handle.api.ui.toast('Select a layer first');
    expect(toastCalls[0]?.opts).toEqual({ source: { id: 'vhs', name: 'VHS effect' } });

    // An extension cannot dress its notice up as the editor's own error, or
    // claim to speak for another extension.
    handle.api.ui.toast('Render failed', { error: true, source: { id: 'timeline', name: 'Timeline' } } as never);
    expect(toastCalls[1]?.opts).toEqual({ error: true, source: { id: 'vhs', name: 'VHS effect' } });
  });

  it('contains asynchronous command failures and ignores disposed callbacks', async () => {
    const { kernel, deps, reported } = harness();
    const handle = createExtensionAPI(kernel, record(), deps);
    const run = vi.fn(async () => { throw new Error('late failure'); });
    handle.api.commands.register({ id: 'async-failure', label: 'Fail', run });
    const callback = kernel.commands.get('async-failure')!.run;
    await expect(Promise.resolve(callback())).resolves.toBeUndefined();
    expect(reported[0]?.error).toEqual(new Error('late failure'));
    handle.disposeAll(); await callback();
    expect(run).toHaveBeenCalledTimes(1);
  });
  it('exposes the frozen surface scoped to the extension id', () => {
    const { kernel, deps } = harness();
    const { api } = createExtensionAPI(kernel, record(), deps);

    expect(api.id).toBe('vhs');
    expect(api.apiVersion).toBe(1);
    expect(api.manifest.name).toBe('VHS effect');
    expect(api.host.pm).toEqual({ legacy: true });
    expect(api.host.state).toBe(deps.state);
    expect(api.ui.controls).toBe(deps.ui.controls);
    expect(api.ui.icon('play')).toContain('data-icon="play"');
    expect(api.keybindings.chordOf(new KeyboardEvent('keydown', { key: 'k', metaKey: true }))).toBe('cmd+k');
  });

  it('forces meta.origin to ext:<id> on every apply', () => {
    const { kernel, deps, applied } = harness();
    const { api } = createExtensionAPI(kernel, record(), deps);

    api.project.apply({ type: 'set_layer', target: 'L1', patch: {} } as never, { label: 'Tint', origin: 'agent' } as never);
    api.project.apply([] as never);

    expect(applied[0]?.meta).toEqual({ label: 'Tint', origin: 'ext:vhs' });
    expect(applied[1]?.meta).toEqual({ origin: 'ext:vhs' });
    expect(api.project.revision()).toBe(3);
  });

  it('owns structured layer definitions and restores the previous provider on dispose', () => {
    const { kernel, deps } = harness();
    const first = createExtensionAPI(kernel, record('first'), deps);
    const second = createExtensionAPI(kernel, record('second'), deps);
    const base = {
      id: 'demo.layer', label: 'Base', version: 1, params: [],
      renderer: { kind: 'fragment' as const, fragment: 'void main(){ fragColor=vec4(1.0); }' }
    };
    first.api.layers.register(base);
    second.api.layers.register({ ...base, label: 'Override' });
    expect(first.api.layers.get('demo.layer')?.label).toBe('Override');
    second.disposeAll();
    expect(first.api.layers.get('demo.layer')?.label).toBe('Base');
  });

  it('namespaces storage per extension', () => {
    const { kernel, deps, bag } = harness();
    const a = createExtensionAPI(kernel, record('a'), deps).api;
    const b = createExtensionAPI(kernel, record('b'), deps).api;

    a.storage.set('seen', 1);
    b.storage.set('seen', 2);
    expect(a.storage.get('seen')).toBe(1);
    expect(bag).toEqual({ a: { seen: 1 }, b: { seen: 2 } });

    a.storage.delete('seen');
    expect(a.storage.get('seen')).toBeUndefined();
    expect(b.storage.get('seen')).toBe(2);
  });

  it('guards every extension callback: errors are logged, attributed, and contained', () => {
    const { kernel, deps, reported } = harness();
    const error = vi.spyOn(console, 'error').mockImplementation(() => {});
    const { api } = createExtensionAPI(kernel, record(), deps);

    api.commands.register({ id: 'boom', label: 'Boom', run: () => { throw new Error('cmd'); } });
    api.status.register({ id: 'st', text: () => { throw new Error('status'); } });
    api.palette.registerProvider(() => { throw new Error('palette'); });
    api.menus.contribute('titlebar:right', () => { throw new Error('menu'); });
    api.panels.register({ id: 'p', title: 'P', build: () => { throw new Error('panel'); } });
    api.on('time', () => { throw new Error('event'); });

    expect(kernel.commands.get('boom')?.run()).toBeUndefined();
    expect(kernel.status.get('st')?.text()).toBeNull();
    expect(kernel.paletteProviders()[0]?.provider('')).toEqual([]);
    expect(kernel.collectMenu('titlebar:right')).toEqual([]);
    expect(() => kernel.panels.get('p')?.build?.(document.createElement('div'), { spec: {} })).not.toThrow();
    expect(() => kernel.events.emit('time', 1)).not.toThrow();

    expect(reported.map((r) => r.id)).toEqual(['vhs', 'vhs', 'vhs', 'vhs', 'vhs', 'vhs']);
    expect(error).toHaveBeenCalled();
    error.mockRestore();
  });

  it('hides commands whose when() throws instead of crashing the palette', () => {
    const { kernel, deps } = harness();
    const error = vi.spyOn(console, 'error').mockImplementation(() => {});
    const { api } = createExtensionAPI(kernel, record(), deps);
    api.commands.register({ id: 'c', label: 'C', run: () => 1, when: () => { throw new Error('when'); } });
    expect(kernel.commands.get('c')?.when?.()).toBe(false);
    error.mockRestore();
  });

  it('rejects malformed registrations up front', () => {
    const { kernel, deps } = harness();
    const { api } = createExtensionAPI(kernel, record(), deps);
    expect(() => api.panels.register({ id: 'x', title: 'X' })).toThrow(/component or build/);
    expect(() => api.commands.register({ id: '', label: 'x', run: () => {} })).toThrow(/id/);
    expect(() => api.theme.register({ id: 't', name: 'T', scheme: 'dark', tokens: { accent: '#f60' } })).toThrow(/must start with --/);
    expect(() => api.palette.registerProvider(null as never)).toThrow(/function/);
    expect(() => api.status.register({ id: 's' } as never)).toThrow(/text/);
  });

  it('disposeAll releases every registration, handler, and onDispose hook', () => {
    const { kernel, deps } = harness();
    const handle = createExtensionAPI(kernel, record(), deps);
    const { api } = handle;
    const bye = vi.fn();

    api.commands.register({ id: 'c', label: 'C', run: () => 1 });
    api.panels.register({ id: 'p', title: 'P', build: () => {} });
    api.keybindings.bind({ key: 'cmd+9', command: 'c' });
    api.effects.register({ id: 'fx', label: 'FX', group: 'G', params: [], frag: 'o=vec4(1.);' });
    api.transitions.register({ id: 'tr', label: 'TR', params: [], frag: 'o=vec4(1.);' });
    api.theme.register({ id: 'th', name: 'Th', scheme: 'dark', tokens: { '--accent': '#f60' } });
    api.status.register({ id: 's', text: () => 'x' });
    api.palette.registerProvider(() => []);
    api.menus.contribute('layer:context', () => [{ label: 'x' }]);
    api.on('layout', () => {});
    api.onDispose(bye);

    expect(kernel.commands.list()).toHaveLength(1);
    handle.disposeAll();

    expect(bye).toHaveBeenCalledTimes(1);
    expect(kernel.commands.list()).toEqual([]);
    expect(kernel.panels.list()).toEqual([]);
    expect(kernel.listBindings()).toEqual([]);
    expect(kernel.effects.list()).toEqual([]);
    expect(kernel.transitions.list()).toEqual([]);
    expect(kernel.themes.list()).toEqual([]);
    expect(kernel.status.list()).toEqual([]);
    expect(kernel.paletteProviders()).toEqual([]);
    expect(kernel.collectMenu('layer:context')).toEqual([]);

    handle.disposeAll();
    expect(bye).toHaveBeenCalledTimes(1);
  });

  it('restores an overridden built-in when the overriding extension is disposed', () => {
    const { kernel, deps } = harness();
    kernel.commands.register('built-in', { id: 'undo', label: 'Undo', run: () => 'built-in' });
    const handle = createExtensionAPI(kernel, record(), deps);
    handle.api.commands.register({ id: 'undo', label: 'Undo (VHS)', run: () => 'ext' });

    expect(kernel.commands.get('undo')?.run()).toBe('ext');
    handle.disposeAll();
    expect(kernel.commands.get('undo')?.run()).toBe('built-in');
  });

  it('routes panel open/close through the backend and lists kernel panel ids', () => {
    const { kernel, deps, opened } = harness();
    const { api } = createExtensionAPI(kernel, record(), deps);
    api.panels.register({ id: 'notes', title: 'Notes', build: () => {} });
    api.panels.open('notes', 'right');
    expect(opened).toEqual(['notes']);
    expect(kernel.panels.ids()).toEqual(['notes']);
    // list() is the backend's view: kernel panels plus whatever legacy PM owns.
    expect(api.panels.list()).toEqual(['viewer']);
    expect(api.panels.isOpen('notes')).toBe(false);
  });

  it('an open() during activate() does not undo a panel the user closed', () => {
    const { kernel, deps, opened } = harness();
    const hidden = new Set(['notes']);
    deps.panelsBackend.isHidden = (id) => hidden.has(id);
    const handle = createExtensionAPI(kernel, record(), deps);
    handle.api.panels.register({ id: 'notes', title: 'Notes', build: () => {} });
    // Boot-time reveal of a panel the user hid: ignored.
    handle.setActivating(true);
    handle.api.panels.open('notes', 'left');
    expect(opened).toEqual([]);
    // A panel that was never hidden still opens during activation.
    handle.api.panels.open('fresh', 'left');
    expect(opened).toEqual(['fresh']);
    // After activation, an explicit open (command, menu) brings the hidden panel back.
    handle.setActivating(false);
    handle.api.panels.open('notes', 'left');
    expect(opened).toEqual(['fresh', 'notes']);
  });

  it('registrations made after disposal are dropped immediately', () => {
    const { kernel, deps } = harness();
    const handle = createExtensionAPI(kernel, record(), deps);
    handle.disposeAll();
    handle.api.commands.register({ id: 'late', label: 'Late', run: () => 1 });
    expect(kernel.commands.has('late')).toBe(false);
  });
});
