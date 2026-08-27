import { afterEach, describe, expect, it } from 'vitest';

import type { PMRegistry } from './registry';
import { install } from './app';

const originalWindow = Object.getOwnPropertyDescriptor(globalThis, 'window');

afterEach(() => {
  if (originalWindow) Object.defineProperty(globalThis, 'window', originalWindow);
  else delete (globalThis as any).window;
});

function appRegistry(withExtensionSurfaces = true): {
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

  const raw = {
    id: 'P1', name: 'Test', w: 1920, h: 1080, fps: 30, dur: 10, bg: '#000000',
    layers: [], assets: {}, markers: [], params: {}, comps: {},
  };
  const projects = new Map<string, any>([[raw.id, raw]]);
  const busHandlers = new Map<string, any[]>();
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
    TYPE_META: {},
    MASK_SHAPES: [],
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
});
