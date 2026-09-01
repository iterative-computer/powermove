import { afterEach, describe, expect, it, vi } from 'vitest';

import type { PMRegistry } from './registry';
import { install } from './app';

const originalWindow = Object.getOwnPropertyDescriptor(globalThis, 'window');

afterEach(() => {
  if (originalWindow) Object.defineProperty(globalThis, 'window', originalWindow);
  else delete (globalThis as any).window;
});

function appRegistry(withExtensionSurfaces = true, bootProject?: any): {
  PM: PMRegistry;
  memory: Map<string, any>;
  listeners: Map<string, any[]>;
  timers: Map<number, any>;
  clearedTimers: number[];
  toasts: string[];
} {
  const memory = new Map<string, any>();
  const listeners = new Map<string, any[]>();
  const timers = new Map<number, any>();
  const clearedTimers: number[] = [];
  const toasts: string[] = [];
  const elements = new Map<string, any>();
  let timerId = 0;

  function element(id = ''): any {
    const classes = new Set<string>();
    return {
      id,
      value: '',
      files: [],
      hidden: false,
      style: {},
      dataset: {},
      textContent: '',
      classList: {
        add: (...names: string[]) => names.forEach(name => classes.add(name)),
        remove: (...names: string[]) => names.forEach(name => classes.delete(name)),
        contains: (name: string) => classes.has(name),
      },
      append(...children: any[]) { this.children = [...(this.children || []), ...children]; },
      appendChild(child: any) { this.append(child); return child; },
      insertBefore(child: any) { this.append(child); return child; },
      querySelector() { return null; },
      setAttribute(name: string, value: any) { this[name] = value; },
      addEventListener() {},
      remove() {},
      replaceWith() {},
      focus() {},
      select() {},
      click() {},
      scrollIntoView() {},
    };
  }

  const documentElement = element('html');
  const body = element('body');
  const document = {
    documentElement,
    body,
    activeElement: null,
    createElement: () => element(),
    getElementById: (id: string) => {
      if (!elements.has(id)) elements.set(id, element(id));
      return elements.get(id);
    },
  };
  const fakeWindow: any = {
    document,
    Blob,
    addEventListener(name: string, listener: any) {
      const bucket = listeners.get(name) || [];
      bucket.push(listener);
      listeners.set(name, bucket);
    },
    removeEventListener() {},
    requestAnimationFrame() { return 1; },
    setTimeout(callback: any) {
      const id = ++timerId;
      timers.set(id, callback);
      return id;
    },
    clearTimeout(id: number) {
      clearedTimers.push(id);
      timers.delete(id);
    },
    setInterval() { return ++timerId; },
  };
  Object.defineProperty(globalThis, 'window', { value: fakeWindow, configurable: true });

  const raw = bootProject || {
    id: 'P1', name: 'Test', w: 1920, h: 1080, fps: 30, dur: 10, bg: '#000000',
    layers: [], assets: {}, markers: [], params: {}, comps: {},
  };
  const projects = new Map<string, any>([[raw.id, raw]]);
  const busHandlers = new Map<string, any[]>();
  let nextId = 0;
  const normalizationCalls: any[] = [];
  const PM: PMRegistry = {
    bootVersion: 0,
    store: {
      get(key: string, fallback: any) { return memory.has(key) ? memory.get(key) : fallback; },
      set(key: string, value: any) { memory.set(key, value); },
      del(key: string) { memory.delete(key); },
    },
    bus: {
      on(name: string, handler: any) {
        const bucket = busHandlers.get(name) || [];
        bucket.push(handler);
        busHandlers.set(name, bucket);
      },
      emit() {},
    },
    invalidate() {},
    Projects: {
      pickBoot: () => raw,
      list: () => [...projects.values()].map(project => ({ id: project.id, name: project.name })),
      get: (id: string) => projects.get(id) || null,
      put: (project: any) => projects.set(project.id, project),
      remove: (id: string) => projects.delete(id),
      getState: () => null,
      putState() {},
      tabs: () => [raw.id],
      markOpen() {},
      markClosed() {},
      rename() {},
    },
    mkProject(input: any) {
      return {
        id: input.id || raw.id, name: 'Untitled', w: 1920, h: 1080, fps: 30, dur: 10,
        bg: '#000000', backgroundFill: null, layers: [], assets: {}, markers: [], work: [0, 10],
        params: {}, comps: {}, ...input,
      };
    },
    normalizeFill: (_fill: any, fallback: string) => ({ stops: [{ color: fallback }] }),
    clamp: (value: number, min: number, max: number) => Math.max(min, Math.min(max, value)),
    TYPE_META: { shape: {}, shader: {}, extension: {} },
    MASK_SHAPES: [],
    uid: (prefix: string) => `${prefix}${++nextId}`,
    P: (value: any) => ({ v: value, kf: [], expr: null }),
    mkLayer(type: string) {
      return {
        type, p: { opacity: { v: 100, kf: [], expr: null } }, fx: [], masks: [],
        transitionIn: null, transitionOut: null, d: type === 'shader' ? { uniforms: {} } : {},
      };
    },
    normalizeKeyframes(keys: any, fallback: any) {
      normalizationCalls.push({ keys, fallback });
      return (Array.isArray(keys) ? keys : []).filter((key: any) =>
        key && Number.isFinite(key.t) && key.t >= 0 && typeof key.v === typeof fallback);
    },
    WS: { init() {}, restoreSnapshot() {}, snapshot: () => ({}), editing: false },
    selectLayers() {},
    L: () => null,
    sel: { layers: [], keys: [], chan: null },
    resolveSelectedKeys: () => [],
    setTime(value: number) { PM.time = value; },
    time: 0,
    TL: { pps: 90, scrollT: 0, scrollY: 0, graph: false },
    hist: { clear() {} },
    assets: {
      restoreProject: async () => ({ stale: false, missing: [] }),
      clear() {},
    },
    Inspector: { refresh() {} },
    Viewer: { layout() {} },
    h(selector: string, ...args: any[]) {
      const node = element();
      const attrs = args[0];
      if (attrs && typeof attrs === 'object' && !Array.isArray(attrs)) Object.assign(node, attrs);
      node.selector = selector;
      return node;
    },
    $(selector: string) {
      const id = selector.replace(/^#/, '');
      if (!elements.has(id)) elements.set(id, element(id));
      return elements.get(id);
    },
    icon: () => element(),
    allProps: () => [],
    GL: { gl: null },
    perf: {},
    round: (value: number) => value,
    PANELS: {},
    Export: { snapshot: () => 'data:image/jpeg;base64,thumb' },
    serialize: () => JSON.stringify(PM.proj),
    toast(message: string) { toasts.push(message); },
  };
  PM.__busHandlers = busHandlers;
  PM.__normalizationCalls = normalizationCalls;

  if (!withExtensionSurfaces) {
    delete PM.TL;
    delete PM.Viewer;
    delete PM.Inspector;
    delete PM.syncShaderUniforms;
  }

  install(PM);
  return { PM, memory, listeners, timers, clearedTimers, toasts };
}

describe('legacy app install', () => {
  it('hydrates extension layers without needing their renderer and preserves safe structured data', () => {
    const bootProject = {
      id: 'P1', name: 'Hybrid', w: 1920, h: 1080, fps: 30, dur: 10, bg: '#000000',
      assets: {}, markers: [], params: {}, comps: {},
      layers: [{
        id: 'L3D', name: '3D', type: 'extension', from: 0, dur: 10,
        d: {
          definition: 'vendor.scene', version: 2, w: 800, h: 600,
          params: { yaw: { v: 25, kf: [{ i: 'k1', t: 1, v: 45 }], expr: null } },
          data: { objects: [{ id: 'cube' }], unsafe: Number.NaN, constructor: { polluted: true } }
        }
      }]
    };
    const { PM } = appRegistry(true, bootProject);
    const layer = PM.proj.layers[0];
    expect(layer.d).toMatchObject({
      definition: 'vendor.scene', version: 2, w: 800, h: 600,
      params: { yaw: { v: 25, kf: [{ t: 1, v: 45 }] } },
      data: { objects: [{ id: 'cube' }], unsafe: null }
    });
    expect(Object.hasOwn(layer.d.data, 'constructor')).toBe(false);
  });

  it('saves named projects with an identity, preserves dirty state through autosave, and supports Save As', async () => {
    const { PM, timers } = appRegistry();
    const saveFile = vi.fn(async (_request: any) => ({ ok: true, path: '/tmp/Test.pmv' }));
    (window as any).powermove = { saveFile };
    expect(await PM.saveProject()).toBe(true);
    expect(saveFile.mock.calls[0]?.[0]).toMatchObject({ projectId: 'P1', name: 'Test.pmv', saveAs: false });
    expect(PM.app.dirty).toBe(false);
    PM.proj.name = 'Edited'; PM.autosave();
    await timers.get(PM.app.saveTimer)();
    expect(PM.app.dirty).toBe(true);
    expect(await PM.saveProject({ saveAs: true })).toBe(true);
    expect(saveFile.mock.calls.at(-1)?.[0]).toMatchObject({ saveAs: true });
    expect(PM.app.dirty).toBe(false);
  });

  it('does not mark cancellation or a failed write saved', async () => {
    const { PM } = appRegistry();
    (window as any).powermove = { saveFile: async () => ({ ok: false, cancelled: true }) };
    PM.autosave();
    expect(await PM.saveProject()).toBe(false);
    expect(PM.app.dirty).toBe(true);
    (window as any).powermove.saveFile = async () => ({ ok: false, cancelled: false, error: 'Disk full' });
    expect(await PM.saveProject()).toBe(false);
    expect(PM.app.dirty).toBe(true);
  });

  it('keeps edits made during a save dirty and suppresses duplicate save dialogs', async () => {
    const { PM } = appRegistry();
    let finish!: (result: any) => void;
    const saveFile = vi.fn(() => new Promise(resolve => { finish = resolve; }));
    (window as any).powermove = { saveFile };
    const saving = PM.saveProject();
    await vi.waitFor(() => expect(saveFile).toHaveBeenCalledOnce());
    expect(await PM.saveProject()).toBe(false);
    PM.proj.name = 'New edit'; PM.autosave();
    finish({ ok: true, path: '/tmp/Test.pmv' });
    expect(await saving).toBe(true);
    expect(PM.app.dirty).toBe(true);
  });

  it('keeps the project open when the close prompt or its Save dialog is cancelled', async () => {
    const { PM } = appRegistry();
    (window as any).powermove = { confirmProjectClose: async () => 'cancel' };
    expect(await PM.prepareToClose()).toBe(false);
    (window as any).powermove = {
      confirmProjectClose: async () => 'save', saveFile: async () => ({ ok: false, cancelled: true })
    };
    expect(await PM.prepareToClose()).toBe(false);
    (window as any).powermove.confirmProjectClose = async () => 'discard';
    expect(await PM.prepareToClose()).toBe(true);
  });

  it('never marks a different project saved when switching during the file dialog', async () => {
    const { PM } = appRegistry();
    let finish!: (result: any) => void;
    const saveFile = vi.fn((_request: any) => new Promise(resolve => { finish = resolve; }));
    (window as any).powermove = { saveFile };
    const saving = PM.saveProject();
    await vi.waitFor(() => expect(saveFile).toHaveBeenCalledOnce());
    PM.proj = { ...PM.proj, id: 'P2', name: 'Another project' }; PM.autosave();
    finish({ ok: true, path: '/tmp/first.pmv' });
    expect(await saving).toBe(true);
    expect(PM.proj.name).toBe('Another project');
    expect(PM.app.dirty).toBe(true);
    expect(PM.projectFileState('P2').path).toBeUndefined();
    expect(PM.projectFileState('P1').path).toBe('/tmp/first.pmv');
  });
  it('boots before viewer, timeline, and inspector extensions activate', () => {
    expect(() => appRegistry(false)).not.toThrow();
  });

  it('persists appearance changes', () => {
    const { PM, memory } = appRegistry();

    PM.theme.apply('dark');

    expect(PM.theme.current).toBe('dark');
    expect(memory.get('theme')).toBe('dark');
  });

  it('includes library mutations in autosave', () => {
    const { PM } = appRegistry();
    const libraryHandlers = PM.__busHandlers.get('library');

    expect(libraryHandlers).toHaveLength(1);
    libraryHandlers[0]();
    expect(PM.app.dirty).toBe(true);
  });

  it('stops a queued import after the active project changes', async () => {
    const { PM, toasts } = appRegistry();
    const originalProject = PM.proj;
    const importing = PM.importFiles([], { project: originalProject });
    PM.proj = { id: 'P2' };

    expect(await importing).toEqual([]);
    expect(toasts.at(-1)).toMatch(/switched projects/);
  });

  it('cancels the APP-owned autosave timer before unload persistence', () => {
    const { PM, listeners, timers, clearedTimers } = appRegistry();
    PM.autosave();
    const saveTimer = PM.app.saveTimer;

    expect(timers.has(saveTimer)).toBe(true);
    listeners.get('beforeunload')?.[0]();
    expect(clearedTimers).toContain(saveTimer);
  });

  it('normalizes every persisted animation surface and preserves missing extension data', () => {
    const raw: any = {
      id: 'P1', name: 'Recovered', w: 1920, h: 1080, fps: 30, dur: 10, bg: '#000000',
      assets: {}, markers: [], params: {}, comps: {},
      layers: [{
        id: 'shape', type: 'shape', name: 'Shape', from: 0, dur: 10, on: true,
        p: { opacity: { v: 100, expr: 12, kf: [{ t: 1, v: 50 }, { t: -1, v: 80 }] } },
        fx: [{ id: 'fx', type: 'extension-effect', on: true, p: { amount: { v: 7, kf: [{ t: 0, v: 8 }] } } }],
        transitionIn: { type: 'extension-transition', dur: 1, p: { amount: { v: 3, kf: [{ t: 0, v: 4 }] } } },
        masks: [], d: {},
      }, {
        id: 'shader', type: 'shader', name: 'Shader', from: 0, dur: 10, on: true,
        p: { opacity: { v: 100, kf: [] } }, fx: [], masks: [],
        d: { uniforms: { strength: { v: 2, kf: [{ t: 0, v: 3 }] } } },
      }],
    };
    const { PM } = appRegistry(false, raw);
    const shape: any = PM.proj.layers[0];
    const shader: any = PM.proj.layers[1];

    expect(shape.p.opacity.kf).toEqual([{ t: 1, v: 50 }]);
    expect(shape.p.opacity.expr).toBeNull();
    expect(shape.fx[0].missing).toBe(true);
    expect(shape.fx[0].p.amount.kf).toHaveLength(1);
    expect(shape.fx[0].p.amount.kf[0]).toMatchObject({ t: 0, v: 8 });
    expect(shape.transitionIn.missing).toBe(true);
    expect(shape.transitionIn.p.amount.kf).toHaveLength(1);
    expect(shape.transitionIn.p.amount.kf[0]).toMatchObject({ t: 0, v: 4 });
    expect(shader.d.uniforms.strength.kf).toHaveLength(1);
    expect(shader.d.uniforms.strength.kf[0]).toMatchObject({ t: 0, v: 3 });
    expect(PM.__normalizationCalls.length).toBeGreaterThanOrEqual(5);
  });
});
