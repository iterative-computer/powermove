import { afterEach, describe, expect, it, vi } from 'vitest';

import { createServicesRegistry } from '../kernel/services';
import type { PMRegistry } from './registry';
import { install } from './app';
import { install as installHistory } from './core/history';
import { unpackProjectFile } from './core/project-file';

const originalWindow = Object.getOwnPropertyDescriptor(globalThis, 'window');

afterEach(() => {
  vi.unstubAllGlobals();
  if (originalWindow) Object.defineProperty(globalThis, 'window', originalWindow);
  else delete (globalThis as any).window;
});

function appRegistry(withExtensionSurfaces = true, bootProject?: any, bootFile?: any): {
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
  const states = new Map<string, any>(bootFile ? [[raw.id, { file: bootFile }]] : []);
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
      getState: (id: string) => states.get(id) || null,
      putState: (id: string, state: any) => states.set(id, state),
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
    hist: { clear() {} },
    assets: {
      restoreProject: async () => ({ stale: false, missing: [] }),
      clear() {},
    },
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
  PM.Kernel = { services: createServicesRegistry() };

  if (withExtensionSurfaces) {
    PM.Kernel.services.register('timeline', { pps: 90, scrollT: 0, scrollY: 0, graph: false });
    PM.Kernel.services.register('viewer', { layout() {} });
    PM.Kernel.services.register('inspector', { refresh() {} });
    PM.Kernel.services.register('shaderHooks', { syncShaderUniforms() {} });
  }

  install(PM);
  return { PM, memory, listeners, timers, clearedTimers, toasts };
}

describe('legacy app install', () => {
  it('compacts oversized provenance when reopening without removing source', () => {
    const { PM } = appRegistry();
    const source = { ...PM.proj, edits: [{ id: 'bulk', summary: ['Cut out subject'], operations: [{ value: 'x'.repeat(200_000) }] }] };
    const layers = JSON.stringify(source.layers);
    const hydrated = PM.hydrateProject(source);
    expect(JSON.stringify(hydrated.layers)).toBe(layers);
    expect(hydrated.edits).toEqual([{ id: 'bulk', summary: ['Cut out subject'], operations: [], payloadOmitted: true }]);
  });

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

  it('closes an unchanged project without a file or a save prompt', async () => {
    const { PM } = appRegistry();
    const confirmProjectClose = vi.fn(async () => 'cancel');
    (window as any).powermove = { confirmProjectClose };
    expect(await PM.confirmCloseProject('P1')).toBe(true);
    expect(confirmProjectClose).not.toHaveBeenCalled();
  });

  it('prompts for edits to a project without a file, but not after reverting them', async () => {
    const { PM } = appRegistry();
    const confirmProjectClose = vi.fn(async () => 'cancel');
    (window as any).powermove = { confirmProjectClose };
    PM.proj.name = 'Edited';
    PM.autosave();
    expect(await PM.confirmCloseProject('P1')).toBe(false);
    expect(confirmProjectClose).toHaveBeenCalledOnce();
    PM.proj.name = 'Test';
    PM.autosave();
    expect(await PM.confirmCloseProject('P1')).toBe(true);
    expect(confirmProjectClose).toHaveBeenCalledOnce();
  });

  it('preserves the original unsaved baseline across recovery', async () => {
    const first = appRegistry().PM;
    await first.confirmCloseProject('P1');
    const file = first.Projects.getState('P1').file;
    expect(file.baselineHash).toMatch(/^[a-f0-9]{64}$/);
    expect(file.savedHash).toBeUndefined();
    const recovered = JSON.parse(JSON.stringify(first.proj));
    recovered.name = 'Recovered edits';
    const { PM } = appRegistry(true, recovered, file);
    const confirmProjectClose = vi.fn(async () => 'cancel');
    (window as any).powermove = { confirmProjectClose };
    expect(await PM.confirmCloseProject('P1')).toBe(false);
    expect(confirmProjectClose).toHaveBeenCalledOnce();
  });

  it('does not prompt after a no-op autosave on a saved project', async () => {
    const { PM, timers } = appRegistry();
    const confirmProjectClose = vi.fn(async () => 'cancel');
    (window as any).powermove = {
      saveFile: async () => ({ ok: true, path: '/tmp/Test.pmv' }), confirmProjectClose,
    };
    await PM.saveProject();
    PM.autosave();
    const recovery = timers.get(PM.app.saveTimer)();
    expect(await PM.confirmCloseProject('P1')).toBe(true);
    await recovery;
    expect(confirmProjectClose).not.toHaveBeenCalled();
  });

  it('does not treat overlapping unchanged comparisons as edits', async () => {
    const { PM } = appRegistry();
    const confirmProjectClose = vi.fn(async () => 'cancel');
    (window as any).powermove = { confirmProjectClose };
    expect(await Promise.all([PM.confirmCloseProject('P1'), PM.confirmCloseProject('P1')])).toEqual([true, true]);
    expect(confirmProjectClose).not.toHaveBeenCalled();
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

  it('waits for an in-flight save before closing instead of rejecting the close', async () => {
    const { PM, toasts } = appRegistry();
    let finish!: (result: any) => void;
    const saveFile = vi.fn(() => new Promise(resolve => { finish = resolve; }));
    const confirmProjectClose = vi.fn(async () => 'cancel');
    (window as any).powermove = { saveFile, confirmProjectClose };
    const saving = PM.saveProject();
    await vi.waitFor(() => expect(saveFile).toHaveBeenCalledOnce());

    let closeFinished = false;
    const closing = PM.confirmCloseProject('P1').then((result: boolean) => {
      closeFinished = true;
      return result;
    });
    await Promise.resolve();
    expect(closeFinished).toBe(false);
    expect(toasts).not.toContain('Please wait for the current save to finish.');

    finish({ ok: true, path: '/tmp/Test.pmv' });
    expect(await saving).toBe(true);
    expect(PM.app.saving).toBe(false);
    expect(await closing).toBe(true);
    expect(confirmProjectClose).not.toHaveBeenCalled();
  });

  it('shows the unsaved prompt after an in-flight save is cancelled', async () => {
    const { PM } = appRegistry();
    PM.proj.name = 'Edited';
    PM.autosave();
    let finish!: (result: any) => void;
    const saveFile = vi.fn(() => new Promise(resolve => { finish = resolve; }));
    const confirmProjectClose = vi.fn(async () => 'discard');
    (window as any).powermove = { saveFile, confirmProjectClose };
    const saving = PM.saveProject();
    await vi.waitFor(() => expect(saveFile).toHaveBeenCalledOnce());
    const closing = PM.confirmCloseProject('P1');

    finish({ ok: false, cancelled: true });
    expect(await saving).toBe(false);
    expect(await closing).toBe(true);
    expect(confirmProjectClose).toHaveBeenCalledOnce();
  });

  it('keeps the project open when the close prompt or its Save dialog is cancelled', async () => {
    const { PM } = appRegistry();
    PM.proj.name = 'Edited';
    PM.autosave();
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

  it('retries a deferred thumbnail without repeatedly saving the document during playback', async () => {
    const { PM, timers } = appRegistry();
    const put = vi.spyOn(PM.Projects, 'put');
    PM.playing = true; PM.autosave();
    await timers.get(PM.app.saveTimer)();
    expect(put).toHaveBeenCalledTimes(1);
    for (let i = 0; i < 3; i++) {
      const id = Math.max(...timers.keys()), callback = timers.get(id);
      timers.delete(id); callback();
    }
    expect(put).toHaveBeenCalledTimes(1);
  });

  it.each(['readback', 'resize'])('defers a thumbnail when playback starts during %s', async stage => {
    const { PM, timers } = appRegistry();
    const captures: Function[] = [];
    let finishResize!: (bitmap: any) => void;
    const bitmap = { width: 320, height: 180, close: vi.fn() };
    const resize = vi.fn(() => new Promise(resolve => { finishResize = resolve; }));
    vi.stubGlobal('createImageBitmap', resize);
    PM.GL.canvas = { width: 640, height: 360, toBlob: vi.fn((callback: Function) => captures.push(callback)) };
    const encode = vi.fn(() => 'data:image/jpeg;base64,preview');
    const create = window.document.createElement;
    window.document.createElement = ((tag: string) => tag === 'canvas'
      ? { getContext: () => ({ drawImage() {} }), toDataURL: encode }
      : create(tag)) as any;
    PM.Projects.upsertMeta = vi.fn();
    PM.autosave(); await timers.get(PM.app.saveTimer)();
    const capture = captures.shift()!;
    if (stage === 'readback') PM.playing = true;
    capture(new Blob(['frame']));
    if (stage === 'resize') { PM.playing = true; finishResize(bitmap); }
    await Promise.resolve(); await Promise.resolve();
    expect(encode).not.toHaveBeenCalled();
    expect(PM.Projects.upsertMeta).not.toHaveBeenCalled();
    if (stage === 'resize') expect(bitmap.close).toHaveBeenCalledOnce();
    else expect(resize).not.toHaveBeenCalled();
    PM.playing = false;
    const id = Math.max(...timers.keys()), retry = timers.get(id);
    timers.delete(id); retry();
    captures.shift()!(new Blob(['new frame']));
    finishResize(bitmap);
    await Promise.resolve(); await Promise.resolve();
    expect(encode).toHaveBeenCalledOnce();
    expect(PM.Projects.upsertMeta).toHaveBeenCalledWith(expect.objectContaining({ id: PM.proj.id, thumb: 'data:image/jpeg;base64,preview' }));
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

  it('rejects multiple replacement files and project files without mutating the project', async () => {
    const { PM, toasts } = appRegistry();
    PM.proj.assets = { original: { id: 'original', name: 'Original.png', kind: 'image' } };
    const before = JSON.stringify(PM.proj);
    const importBatch = PM.assets.importBatch = vi.fn();
    await PM.importFiles([{ name: 'A.png' }, { name: 'B.png' }], { replaceAssetId: 'original', sequence: false });
    expect(toasts.at(-1)).toContain('Choose one file or one image sequence');
    await PM.importFiles([{ name: 'Another.pmv' }], { replaceAssetId: 'original' });
    expect(toasts.at(-1)).toContain('Choose a media file');
    expect(importBatch).not.toHaveBeenCalled();
    expect(JSON.stringify(PM.proj)).toBe(before);
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

it('saves real undo and redo history in the native file and local session', async () => {
  const { PM, toasts } = appRegistry();
  installHistory(PM);
  PM.pause = vi.fn();
  PM.rasterClear = vi.fn();
  PM.WS.activate = vi.fn();
  PM.touch = vi.fn();
  PM.replaceProject = (next: any) => { PM.proj = next; };
  PM.hist.do('Rename one', () => { PM.proj.name = 'One'; });
  PM.hist.do('Rename two', () => { PM.proj.name = 'Two'; });
  PM.hist.undo();
  const saveFile = vi.fn(async (_request: any) => ({ ok: true, path: '/tmp/Test.pmv' }));
  (window as any).powermove = { saveFile };
  expect(await PM.saveProject()).toBe(true);
  const saved = unpackProjectFile(saveFile.mock.calls[0]![0].data);
  expect(saved.history.index).toBe(0);
  expect(saved.history.entries.map((entry: any) => entry.label)).toEqual(['Rename one', 'Rename two']);
  expect(PM.Projects.getState('P1').history).toEqual(saved.history);
  (window as any).powermove.openProjectFile = async () => ({ ok: true, path: '/tmp/Test.pmv', projectId: 'reopened', data: saveFile.mock.calls[0]![0].data });
  await PM.openProject();
  expect(toasts).not.toEqual(expect.arrayContaining([expect.stringContaining('Could not open')]));
  expect(PM.proj.id).toBe('reopened');
  expect(PM.hist.canUndo()).toBe(true);
  expect(PM.hist.canRedo()).toBe(true);
  PM.hist.redo();
  expect(PM.proj.name).toBe('Two');
  PM.hist.undo();
  expect(PM.proj.name).toBe('One');
});
