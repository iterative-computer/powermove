import { installBridgeForTests, resetBridgeForTests } from '../kernel/bridge';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { createServicesRegistry } from '../kernel/services';
import type { PMRegistry } from './registry';
import { install } from './app';
import { install as installHistory } from './core/history';
import { install as installProjects } from './core/projects';
import { unpackProjectFile } from './core/project-file';
import { makePM } from './__tests__/make-pm';
import { install as installAnim } from './core/anim';
import { sceneProperties } from '../core/scene3d/schema';
import * as fingerprints from './core/project-fingerprint';
import * as serialization from './core/serialize-async';
import { sha256Hex } from '../../../shared/sha256';

const originalWindow = Object.getOwnPropertyDescriptor(globalThis, 'window');

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  resetBridgeForTests();
  if (originalWindow) Object.defineProperty(globalThis, 'window', originalWindow);
  else delete (globalThis as any).window;
});

function appRegistry(withExtensionSurfaces = true, bootProject?: any, bootFile?: any, bridge?: any): {
  PM: PMRegistry;
  memory: Map<string, any>;
  listeners: Map<string, any[]>;
  timers: Map<number, any>;
  clearedTimers: number[];
  toasts: string[];
  frames: Array<() => unknown>;
} {
  const frames: Array<() => unknown> = [];
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
    powermove: bridge,
    addEventListener(name: string, listener: any) {
      const bucket = listeners.get(name) || [];
      bucket.push(listener);
      listeners.set(name, bucket);
    },
    removeEventListener() {},
    requestAnimationFrame(callback: () => unknown) { frames.push(callback); return frames.length; },
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
  installBridgeForTests(bridge);

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
      destroy: (id: string) => states.delete(id),
      unwrap: (raw: any) => raw?.proj || raw,
      getState: (id: string) => states.get(id) || null,
      putState: (id: string, state: any) => states.set(id, state),
      openProjects: () => [raw.id],
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
    normalizeKeyframes(keys: any, fallback: any, _fps: any, minTime = 0) {
      normalizationCalls.push({ keys, fallback });
      return (Array.isArray(keys) ? keys : []).filter((key: any) =>
        key && Number.isFinite(key.t) && key.t >= minTime && typeof key.v === typeof fallback);
    },
    WS: { init() {}, restoreSnapshot() {}, snapshot: () => ({}), get: (id: string) => ({ id }), activate() {}, editing: false },
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
  return { PM, memory, listeners, timers, clearedTimers, toasts, frames };
}

async function reopenWithPendingMedia() {
  const { PM } = appRegistry();
  PM.pause = vi.fn(); PM.rasterClear = vi.fn(); PM.touch = vi.fn();
  PM.replaceProject = (next: any) => { PM.proj = next; };
  await Promise.resolve();
  PM.bus.emit = (name: string, ...args: any[]) => {
    for (const handler of PM.__busHandlers.get(name) || []) handler(...args);
  };
  const saveFile = vi.fn(async (_request: any) => ({ ok: true, path: '/tmp/Test.pmv' }));
  installBridgeForTests({ saveFile } as any);
  expect(await PM.saveProject()).toBe(true);
  const data = saveFile.mock.calls[0]![0].data;
  let release!: () => void;
  PM.assets.restoreProject = () => new Promise(resolve => { release = () => resolve({ stale: false, missing: [] }); });
  installBridgeForTests({ openProjectFile: async () => ({ ok: true, path: '/tmp/Test.pmv', projectId: 'reopened', data }) } as any);
  await PM.openProject();
  expect(PM.app.dirty).toBe(false);
  return { PM, release };
}

describe('legacy app install', () => {
  it('keeps a reopened saved document clean after runtime-only media restoration notifications', async () => {
    const { PM, release } = await reopenWithPendingMedia();
    const before = JSON.stringify(PM.proj), savedHash = PM.projectFileState(PM.proj.id).savedHash;
    release(); await PM.projectAssetsReady;
    expect(JSON.stringify(PM.proj)).toBe(before);
    expect(PM.app.dirty).toBe(false);
    expect(PM.projectFileState(PM.proj.id).savedHash).toBe(savedHash);
  });
  it.each(['user edit', 'persisted media metadata repair'])('keeps a %s dirty when restoration finishes', async kind => {
    const { PM, release } = await reopenWithPendingMedia();
    const savedHash = PM.projectFileState(PM.proj.id).savedHash;
    if (kind === 'user edit') PM.proj.name = 'Edited during loading';
    else PM.proj.assets.repaired = { id: 'repaired', kind: 'image', format: 'svg', poster: true };
    if (kind === 'user edit') PM.autosave();
    release(); await PM.projectAssetsReady;
    expect(PM.app.dirty).toBe(true);
    expect(PM.projectFileState(PM.proj.id).savedHash).toBe(savedHash);
    if (kind === 'user edit') expect(PM.proj.name).toBe('Edited during loading');
    else expect(PM.proj.assets.repaired.format).toBe('svg');
  });
  it('keeps a newer edit dirty when it arrives during the restoration content comparison', async () => {
    const { PM, release } = await reopenWithPendingMedia();
    const savedHash = PM.projectFileState(PM.proj.id).savedHash;
    const digest = crypto.subtle.digest.bind(crypto.subtle);
    let finishHash!: () => void;
    const hashing = vi.spyOn(crypto.subtle, 'digest').mockImplementationOnce(async (...args) => {
      await new Promise<void>(resolve => { finishHash = resolve; });
      return digest(...args);
    });
    try {
      release();
      await vi.waitFor(() => expect(hashing).toHaveBeenCalled());
      PM.proj.name = 'Newer edit during comparison'; PM.autosave();
      finishHash(); await PM.projectAssetsReady;
      expect(PM.app.dirty).toBe(true);
      expect(PM.proj.name).toBe('Newer edit during comparison');
      expect(PM.projectFileState(PM.proj.id).savedHash).toBe(savedHash);
    } finally { hashing.mockRestore(); }
  });
  it('does not clear another tab’s edits when an old restoration finishes', async () => {
    const { PM, release } = await reopenWithPendingMedia();
    const ready = PM.projectAssetsReady;
    PM.proj = PM.mkProject({ id: 'other', name: 'Other unsaved project' });
    PM.projectFileState('other');
    PM.proj.name = 'Other edits'; PM.autosave();
    release(); await ready;
    expect(PM.proj.name).toBe('Other edits');
    expect(PM.app.dirty).toBe(true);
  });

  it('ignores runtime preview readiness while still autosaving asset metadata changes', () => {
    const { PM } = appRegistry();
    const previous = { timer: PM.app.saveTimer, dirty: PM.app.dirty };
    const handlers = PM.__busHandlers.get('assets');
    handlers.forEach((run: any) => run({ runtimeOnly: true }));
    expect(PM.app.saveTimer).toBe(previous.timer);
    expect(PM.app.dirty).toBe(previous.dirty);
    PM.proj.assets.source = { id: 'source', name: 'Edited media' };
    handlers.forEach((run: any) => run());
    expect(PM.app.saveTimer).not.toBe(previous.timer);
    expect(PM.app.dirty).toBe(true);
  });

  it('checks autosave file status once while retaining exact Close comparisons', async () => {
    const { PM, timers } = appRegistry();
    await vi.waitFor(() => expect(PM.projectFileState().dirty).toBe(false));
    PM.Projects.recoverAsync = vi.fn(async () => true);
    const stringify = vi.spyOn(serialization, 'stringifyAsync');
    PM.autosave(); await timers.get(PM.app.saveTimer)();
    expect(stringify).toHaveBeenCalledTimes(1);
    stringify.mockClear();
    expect(await PM.confirmCloseProject(PM.proj.id)).toBe(true);
    expect(stringify).toHaveBeenCalledTimes(2);
  });

  it.each(['name', 'assets', 'composition', 'animation', 'history', 'project'])('abandons an autosave comparison when %s changes during hashing', async change => {
    const { PM, timers } = appRegistry();
    await vi.waitFor(() => expect(PM.projectFileState().dirty).toBe(false));
    let editVersion = 1, editing = false;
    PM.animVersion = () => editVersion;
    PM.hist.busy = () => editing;
    PM.Projects.recoverAsync = vi.fn(async () => true);
    let finish!: (hash: string) => void;
    const fingerprint = vi.spyOn(fingerprints, 'projectFingerprint').mockImplementation(() => new Promise(resolve => { finish = resolve; }));
    const emit = vi.spyOn(PM.bus, 'emit');
    const project = PM.proj, baseline = PM.projectFileState().baselineHash;
    PM.autosave();
    const saving = timers.get(PM.app.saveTimer)();
    await vi.waitFor(() => expect(fingerprint).toHaveBeenCalledOnce());
    if (change === 'name') { project.name = 'Newer edit'; PM.autosave(); }
    else if (change === 'assets') {
      project.assets.source = { id: 'source', name: 'Newer metadata' };
      PM.__busHandlers.get('assets').forEach((run: any) => run());
    } else if (change === 'composition') {
      project.comps.sub = { layers: [], dur: 2 };
      PM.__busHandlers.get('project').forEach((run: any) => run());
    } else if (change === 'animation') { project.name = 'Newer edit'; editVersion++; }
    else if (change === 'history') { project.name = 'Newer edit'; editing = true; }
    else PM.proj = { ...project, name: 'Replacement project' };
    emit.mockClear(); finish(baseline); await saving;
    expect(PM.projectFileState().dirty).toBe(true);
    expect(emit.mock.calls.some(([event]: any[]) => event === 'project:recovered')).toBe(false);
  });

  it('awaits recovery and abandons a superseded autosave before writing its session', async () => {
    const { PM, timers } = appRegistry();
    let finish!: (saved: boolean) => void;
    let current!: () => boolean;
    PM.Projects.recoverAsync = vi.fn((_project, _thumb, guard) => {
      current = guard; return new Promise<boolean>(resolve => finish = resolve);
    });
    const putState = vi.spyOn(PM.Projects, 'putState');
    PM.store.flush = vi.fn(async () => undefined);
    PM.autosave();
    const saving = timers.get(PM.app.saveTimer)();
    expect(current()).toBe(true);
    expect(putState).not.toHaveBeenCalled();
    PM.proj.name = 'Newer edit'; PM.autosave();
    expect(current()).toBe(false);
    finish(true); await saving;
    expect(putState).not.toHaveBeenCalled();
    expect(PM.store.flush).not.toHaveBeenCalled();
  });
  it('opens a project delivered by the external-open subscription', async () => {
    let externalOpen!: (result: any) => Promise<void>;
    const onProjectOpenExternal = vi.fn((callback: typeof externalOpen) => {
      externalOpen = callback;
      return () => {};
    });
    const { PM, toasts } = appRegistry(true, undefined, undefined, { onProjectOpenExternal });
    PM.pause = vi.fn();
    PM.rasterClear = vi.fn();
    PM.WS.activate = vi.fn();
    PM.touch = vi.fn();
    PM.replaceProject = (next: any) => { PM.proj = next; };
    const hide = vi.fn();
    PM.ProjectsScreen = { hide };
    const document = { proj: { ...PM.proj, id: 'from-finder', name: 'Finder Project' } };

    await externalOpen({
      ok: true,
      path: '/tmp/Finder Project.pmv',
      projectId: 'from-finder',
      data: new TextEncoder().encode(JSON.stringify(document))
    });

    expect(onProjectOpenExternal).toHaveBeenCalledOnce();
    expect(PM.proj.id).toBe('from-finder');
    expect(hide).toHaveBeenCalledOnce();
    expect(toasts).toContain('Opened Finder Project.pmv');
  });

  it('reopens negative strip starts and their earlier local keys, including nested compositions', () => {
    const { PM } = appRegistry();
    const layer = {
      id: 'early', type: 'solid', from: -6, dur: 8,
      p: { opacity: { v: 100, kf: [{ i: 'before', t: -1, v: 0 }, { i: 'after', t: 1, v: 100 }] } },
    };
    const source = { ...PM.proj, layers: [layer], comps: {
      nested: { ...PM.proj, layers: [{ ...layer, id: 'nested-early', p: {} }] },
    } };
    const restored = PM.hydrateProject(source);
    expect(restored.layers[0].from).toBe(-6);
    expect(restored.layers[0].p.opacity.kf.map((key: any) => key.t)).toEqual([-1, 1]);
    expect(restored.comps.nested.layers[0].from).toBe(-6);
  });

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
  installBridgeForTests((window as any).powermove);
    expect(await PM.confirmCloseProject('P1')).toBe(true);
    expect(confirmProjectClose).not.toHaveBeenCalled();
  });

  it('prompts for edits to a project without a file, but not after reverting them', async () => {
    const { PM } = appRegistry();
    const confirmProjectClose = vi.fn(async () => 'cancel');
    (window as any).powermove = { confirmProjectClose };
  installBridgeForTests((window as any).powermove);
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
  installBridgeForTests((window as any).powermove);
    expect(await PM.confirmCloseProject('P1')).toBe(false);
    expect(confirmProjectClose).toHaveBeenCalledOnce();
  });

  it('does not prompt after a no-op autosave on a saved project', async () => {
    const { PM, timers } = appRegistry();
    const confirmProjectClose = vi.fn(async () => 'cancel');
    (window as any).powermove = {
      saveFile: async () => ({ ok: true, path: '/tmp/Test.pmv' }), confirmProjectClose,
    };
  installBridgeForTests((window as any).powermove);
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
  installBridgeForTests((window as any).powermove);
    expect(await Promise.all([PM.confirmCloseProject('P1'), PM.confirmCloseProject('P1')])).toEqual([true, true]);
    expect(confirmProjectClose).not.toHaveBeenCalled();
  });

  it('saves named projects with an identity, preserves dirty state through autosave, and supports Save As', async () => {
    const { PM, timers } = appRegistry();
    const saveFile = vi.fn(async (_request: any) => ({ ok: true, path: '/tmp/Test.pmv' }));
    (window as any).powermove = { saveFile };
  installBridgeForTests((window as any).powermove);
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

  it('shows saving immediately and completes the same progress notice only after the write', async () => {
    const { PM } = appRegistry();
    const toast = PM.toast = vi.fn();
    PM.dismissToast = vi.fn();
    let release!: (result: any) => void;
    const saveFile = vi.fn(() => new Promise(resolve => { release = resolve; }));
    (window as any).powermove = { saveFile };
  installBridgeForTests((window as any).powermove);
    const saving = PM.saveProject();
    expect(toast).toHaveBeenCalledWith('Saving Test.pmv', 2200, expect.objectContaining({ key: 'project-save', sticky: true, progress: null }));
    await vi.waitFor(() => expect(saveFile).toHaveBeenCalledOnce());
    expect(toast.mock.calls.some(call => call[2]?.completed)).toBe(false);
    release({ ok: true, path: '/tmp/Test.pmv' });
    expect(await saving).toBe(true);
    expect(toast).toHaveBeenLastCalledWith('Saved Test.pmv', 2200, expect.objectContaining({ key: 'project-save', progress: 1, completed: true }));
    (window as any).powermove.saveFile = async () => ({ ok: false, cancelled: true });
    await PM.saveProject();
    expect(PM.dismissToast).toHaveBeenCalledWith('project-save');
  });

  it('does not mark cancellation or a failed write saved', async () => {
    const { PM } = appRegistry();
    (window as any).powermove = { saveFile: async () => ({ ok: false, cancelled: true }) };
  installBridgeForTests((window as any).powermove);
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
  installBridgeForTests((window as any).powermove);
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
  installBridgeForTests((window as any).powermove);
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
  installBridgeForTests((window as any).powermove);
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
  installBridgeForTests((window as any).powermove);
    expect(await PM.prepareToClose()).toBe(false);
    (window as any).powermove = {
      confirmProjectClose: async () => 'save', saveFile: async () => ({ ok: false, cancelled: true })
    };
  installBridgeForTests((window as any).powermove);
    expect(await PM.prepareToClose()).toBe(false);
    (window as any).powermove.confirmProjectClose = async () => 'discard';
    expect(await PM.prepareToClose()).toBe(true);
  });

  it('discards a never-saved project on window close, including recovery and later flushes', async () => {
    const { PM, memory, listeners, timers } = appRegistry();
    installProjects(PM);
    PM.proj.name = 'Discard me';
    PM.autosave();
    await timers.get(PM.app.saveTimer)();
    PM.Projects.putState('P1', { history: { entries: [] }, workspace: {} });
    memory.set('projectJournal.P1', []);
    memory.set('autosave', { proj: PM.proj });
    expect(PM.Projects.get('P1')).not.toBeNull();
    (window as any).powermove = { confirmProjectClose: async () => 'discard' };
    installBridgeForTests((window as any).powermove);

    expect(await PM.prepareToClose()).toBe(true);
    await PM.flushProject();
    for (const listener of listeners.get('beforeunload') || []) listener();
    PM.autosave();
    expect(PM.Projects.list()).toEqual([]);
    expect(PM.Projects.get('P1')).toBeNull();
    expect(PM.Projects.getState('P1')).toBeNull();
    expect(memory.has('projectJournal.P1')).toBe(false);
    expect(memory.has('autosave')).toBe(false);
  });

  it('keeps a never-saved project for the restored session when Don’t Save answers a quit', async () => {
    const { PM, timers } = appRegistry();
    installProjects(PM);
    PM.proj.name = 'Keep me';
    PM.autosave();
    await timers.get(PM.app.saveTimer)();
    const confirmProjectClose = vi.fn(async () => 'discard');
    (window as any).powermove = { confirmProjectClose };
    installBridgeForTests((window as any).powermove);

    expect(await PM.prepareToClose({ quitting: true })).toBe(true);
    expect(confirmProjectClose).toHaveBeenCalledOnce();
    await PM.flushProject();
    PM.autosave();
    expect(PM.Projects.get('P1')?.name).toBe('Keep me');
    expect(PM.Projects.list().map((project: any) => project.id)).toEqual(['P1']);

    // The answer does not linger: closing the window afterwards still discards.
    expect(await PM.prepareToClose()).toBe(true);
    expect(PM.Projects.get('P1')).toBeNull();
  });

  it('removes a never-saved tab without recapturing it when showing its neighbour', async () => {
    const { PM, listeners } = appRegistry();
    installProjects(PM);
    PM.Projects.put(PM.proj);
    PM.pause = vi.fn();
    PM.rasterClear = vi.fn();
    PM.WS.activate = vi.fn();
    PM.touch = vi.fn();
    listeners.get('pm-open-project')![0]({ detail: PM.mkProject({ id: 'P2', name: 'New project' }) });
    PM.proj.name = 'Edited new project';
    PM.autosave();
    (window as any).powermove = { confirmProjectClose: async () => 'discard' };
    installBridgeForTests((window as any).powermove);

    expect(await PM.Tabs.close('P2')).toBe(true);
    expect(PM.proj.id).toBe('P1');
    expect(PM.Tabs.list()).toEqual(['P1']);
    expect(PM.Projects.get('P2')).toBeNull();
    expect(PM.Projects.getState('P2')).toBeNull();
    expect(PM.Projects.list().map((project: any) => project.id)).toEqual(['P1']);
  });

  it('preserves every project if a later tab cancels the window close', async () => {
    const { PM, listeners } = appRegistry();
    PM.pause = vi.fn();
    PM.rasterClear = vi.fn();
    PM.WS.activate = vi.fn();
    PM.touch = vi.fn();
    PM.proj.name = 'Edited first'; PM.autosave();
    listeners.get('pm-open-project')![0]({ detail: PM.mkProject({ id: 'P2', name: 'Second' }) });
    PM.proj.name = 'Edited second'; PM.autosave();
    const confirmProjectClose = vi.fn().mockResolvedValueOnce('discard').mockResolvedValueOnce('cancel');
    (window as any).powermove = { confirmProjectClose };
    installBridgeForTests((window as any).powermove);

    expect(await PM.prepareToClose()).toBe(false);
    expect(confirmProjectClose).toHaveBeenCalledTimes(2);
    await PM.flushProject();
    expect(PM.Projects.list().map((project: any) => project.id)).toEqual(['P1', 'P2']);
    expect(PM.Tabs.list()).toEqual(['P1', 'P2']);
  });

  it('keeps a previously saved project in Recents when choosing Don’t Save', async () => {
    const { PM } = appRegistry(true, undefined, { path: '/tmp/Test.pmv', savedHash: 'previous-save' });
    (window as any).powermove = { confirmProjectClose: async () => 'discard' };
    installBridgeForTests((window as any).powermove);
    expect(await PM.prepareToClose()).toBe(true);
    await PM.flushProject();
    expect(PM.Projects.get('P1')).not.toBeNull();
    expect(PM.projectFileState('P1').path).toBe('/tmp/Test.pmv');
  });

  it('never marks a different project saved when switching during the file dialog', async () => {
    const { PM } = appRegistry();
    let finish!: (result: any) => void;
    const saveFile = vi.fn((_request: any) => new Promise(resolve => { finish = resolve; }));
    (window as any).powermove = { saveFile };
  installBridgeForTests((window as any).powermove);
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

  /** The editor's first-frame hooks, observed through the services and bus it calls. */
  function firstFrameProbe(PM: PMRegistry, frames: Array<() => unknown>) {
    const services = PM.Kernel.services;
    const layout = vi.fn(), frameView = vi.fn(), refresh = vi.fn(), init = vi.fn();
    Object.assign(services.get('viewer'), { layout });
    Object.assign(services.get('timeline'), { frameView });
    Object.assign(services.get('inspector'), { refresh });
    PM.SpatialAssistant = { init };
    // Boot's asset restore also refreshes and announces 'assets'; only the
    // first frame's own announcements are recorded.
    const emitted: string[] = [];
    PM.bus.emit = (name: string) => { if (['layers', 'sel', 'projects:open'].includes(name)) emitted.push(name); };
    const runFrames = () => { while (frames.length) frames.shift()!(); };
    return { layout, frameView, refresh, init, emitted, runFrames };
  }

  it('sets the editor up in the first frame when the document can answer', () => {
    const { PM, frames } = appRegistry();
    PM.EngineDoc = { enabled: true, strict: true, ready: true };
    const probe = firstFrameProbe(PM, frames);
    probe.runFrames();
    expect(probe.layout).toHaveBeenCalledOnce();
    expect(probe.frameView).toHaveBeenCalledOnce();
    expect(probe.refresh).toHaveBeenCalledOnce();
    expect(probe.init).toHaveBeenCalledOnce();
    expect(probe.emitted).toEqual(['layers', 'sel', 'projects:open']);
  });

  it('finishes the first-frame setup once when a Retry recovers a failed document load', async () => {
    const { PM, frames } = appRegistry();
    const listeners = new Set<() => void>();
    let loading: Promise<boolean> | null = null;
    const doc: any = {
      enabled: true, strict: true, ready: false, whenReady: Promise.resolve(false),
      settled: () => loading ?? Promise.resolve(doc.ready),
      showFailure: vi.fn(),
      onReady(fn: () => void) { listeners.add(fn); return () => { listeners.delete(fn); }; },
      retry() {
        loading = Promise.resolve().then(() => {
          doc.ready = true; loading = null;
          for (const fn of [...listeners]) fn();
          return true;
        });
        return loading;
      },
    };
    PM.EngineDoc = doc;
    const probe = firstFrameProbe(PM, frames);
    probe.runFrames();
    await new Promise(resolve => setTimeout(resolve, 0));
    // The load failed: layout ran, nothing that reads the document did, and the
    // named failure was shown rather than the frame disappearing.
    expect(probe.layout).toHaveBeenCalledOnce();
    expect(probe.frameView).toHaveBeenCalledOnce();
    expect(probe.init).not.toHaveBeenCalled();
    // Boot's own asynchronous work may announce the projects screen; the
    // panels were not told about layers or selection.
    expect(probe.emitted.filter(name => name !== 'projects:open')).toEqual([]);
    const refreshes = probe.refresh.mock.calls.length;
    const before = probe.emitted.length;
    expect(doc.showFailure).toHaveBeenCalled();
    expect(listeners.size).toBe(1);

    expect(await doc.retry()).toBe(true);
    probe.runFrames();
    expect(probe.init).toHaveBeenCalledOnce();
    expect(probe.refresh).toHaveBeenCalledTimes(refreshes + 1);
    expect(probe.emitted.slice(before)).toEqual(['layers', 'sel', 'projects:open']);
    expect(listeners.size).toBe(0);
    // A later revival does not set the editor up a second time.
    doc.ready = false;
    await doc.retry();
    probe.runFrames();
    expect(probe.init).toHaveBeenCalledOnce();
    expect(probe.emitted.slice(before)).toEqual(['layers', 'sel', 'projects:open']);
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

  it.each([true, false])('matches source settings only for an Open import (%s)', async matchSource => {
    const { PM } = appRegistry();
    const asset = { id: 'clip', kind: 'video', name: 'Clip.mp4', w: 720, h: 1280, fps: 30000 / 1001, dur: 2.002 };
    PM.assetKind = () => 'video';
    PM.assets.importBatch = vi.fn(async () => [{ status: 'created', asset, persisted: true }]);
    let settingsAtInsertion: any;
    PM.commandForAsset = vi.fn((_id, at) => {
      settingsAtInsertion = { w: PM.proj.w, h: PM.proj.h, fps: PM.proj.fps, dur: PM.proj.dur };
      return { type: 'add_layer', from: at };
    });
    PM.Edit = { apply: vi.fn() };
    await PM.importFiles([{ name: 'Clip.mp4' }], { matchSource, placement: { at: 0 }, sequence: false });
    expect(PM.Edit.apply).toHaveBeenCalledOnce();
    expect(settingsAtInsertion).toEqual(matchSource
      ? { w: 720, h: 1280, fps: 30000 / 1001, dur: 2.002 }
      : { w: 1920, h: 1080, fps: 30, dur: 10 });
    if (matchSource) {
      expect(PM.proj.work).toEqual([0, 2.002]);
      expect(PM.proj.exportDefaults.fps).toBe(30000 / 1001);
    }
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

    expect(shape.p.opacity.kf.map(({ t, v }: any) => ({ t, v }))).toEqual([{ t: 1, v: 50 }, { t: -1, v: 80 }]);
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

it('keeps effect parameter key ids across repeated hydration, including unknown parameters', () => {
  const { PM } = appRegistry();
  PM.FX = { custom: { params: [{ k: 'amount', def: 1 }, { k: 'color', def: '#ffffff' }, { k: 'enabled', def: true }, { k: 'primitive', def: 1 }] } };
  const prop = (v: any, i: string) => ({ v, kf: [{ i, t: 0, v }], expr: null });
  const raw = PM.mkProject({ layers: [{ id: 'effect-layer', type: 'shape', from: 0, dur: 2, p: {}, d: {},
    fx: [{ id: 'effect', type: 'custom', p: { amount: prop(2, 'amount-key'), color: prop('#ff0000', 'color-key'),
      enabled: prop(false, 'boolean-key'), extra: prop(3, 'unknown-key'), primitive: 7 } }] }] });
  const hydrated = PM.hydrateProject(raw);
  expect(Object.values<any>(hydrated.layers[0].fx[0].p).flatMap(p => p.kf.map((key: any) => key.i)))
    .toEqual(['amount-key', 'color-key', 'boolean-key', 'unknown-key']);
  expect(hydrated.layers[0].fx[0].p.primitive.v).toBe(7);
  const before = JSON.stringify(hydrated);
  expect(JSON.stringify(PM.hydrateProject(JSON.parse(before)))).toBe(before);
});

it('keeps the two-stage effect parameter hydration for null, missing and mismatched values', () => {
  const { PM } = appRegistry();
  PM.Ease = makePM('core/easing').Ease;
  installAnim(PM);
  PM.FX = { custom: { params: [{ k: 'nullValue', def: 2 }, { k: 'primitiveNull', def: 2 }, { k: 'missingColor', def: '#ffffff' },
    { k: 'missingToggle', def: true }, { k: 'stringNumber', def: 2 }, { k: 'numberColor', def: '#ffffff' }, { k: 'animated', def: 2 }] } };
  const keys = (a: any, b: any, ids?: [string, string]) => [{ ...(ids ? { i: ids[0] } : {}), t: 0, v: a }, { ...(ids ? { i: ids[1] } : {}), t: 1, v: b }];
  const raw = PM.mkProject({ layers: [{ id: 'effect-layer', type: 'shape', from: 0, dur: 2, p: {}, d: {},
    fx: [{ id: 'effect', type: 'custom', p: {
      nullValue: { v: null, kf: [], expr: null },
      primitiveNull: null,
      missingColor: { kf: keys('#ff0000', '#00ff00') },
      missingToggle: { kf: keys(true, false) },
      stringNumber: { v: '5', kf: keys(1, 5) },
      numberColor: { v: 3, kf: keys('#ff0000', '#00ff00') },
      animated: { v: 4, kf: keys(1, 5, ['first-key', 'second-key']), expr: null },
    } }] }] });
  const hydrated = PM.hydrateProject(raw);
  const params = hydrated.layers[0].fx[0].p;
  const summary = Object.fromEntries(Object.entries<any>(params).map(([key, prop]) =>
    [key, { v: prop.v, kf: prop.kf.map((frame: any) => frame.v) }]));
  // Today's app: the loose pass types keys by the saved value (or 0), then the
  // definition's pass types the value by def. Keys the loose pass drops stay dropped.
  expect(summary).toEqual({
    nullValue: { v: 0, kf: [] },
    primitiveNull: { v: 0, kf: [] },
    missingColor: { v: '#ffffff', kf: [] },
    missingToggle: { v: true, kf: [] },
    stringNumber: { v: 2, kf: [] },
    numberColor: { v: '#ffffff', kf: [] },
    animated: { v: 4, kf: [1, 5] },
  });
  expect(params.animated.kf.map((key: any) => key.i)).toEqual(['first-key', 'second-key']);
  const before = JSON.stringify(hydrated);
  expect(JSON.stringify(PM.hydrateProject(JSON.parse(before)))).toBe(before);
});

it('hydrates legacy scene animation before either engine evaluates it', () => {
  const { PM } = appRegistry();
  PM.Ease = makePM('core/easing').Ease;
  installAnim(PM);
  const prop = (a: any = 0, b: any = 90, ids = true) => ({ v: a, kf: [{ ...(ids ? { i: PM.uid('k') } : {}), t: 0, v: a }, { ...(ids ? { i: PM.uid('k') } : {}), t: 1, v: b }], expr: null });
  const layer = () => ({ id: PM.uid('L'), type: 'extension', from: 0, dur: 2, p: {}, fx: [], masks: [],
    d: { definition: 'powermove.3d.scene', data: { scene: {
      objects: [{ id: 'box', name: 'Box', p: { ry: prop(), visible: prop(true, false, false) }, material: { p: { roughness: prop(), color: prop('#000000', '#ffffff', false) } } }],
      lights: [{ id: 'sun', p: { x: prop(0, 90, false), color: prop('#000000', '#ffffff'), visible: prop(true, false) } }], camera: { p: { x: prop() } }, environment: { p: { exposure: prop() } },
    } } } });
  const raw = PM.mkProject({ id: 'scene-project', layers: [layer()], comps: {
    nested: { id: 'nested', w: 64, h: 64, fps: 30, dur: 2, layers: [layer()] },
  } });
  const expected = [raw, raw.comps.nested].map(comp => sceneProperties(comp.layers[0]).map(({ prop: channel }) =>
    [0.5, 0.25].map(time => PM.evalKfs(JSON.parse(JSON.stringify(channel.kf)), time, typeof channel.v === 'string'))));
  const project = PM.hydrateProject(raw);
  const before = JSON.stringify(project);
  for (const [index, comp] of [project, project.comps.nested].entries()) {
    for (const [channelIndex, { prop: channel }] of sceneProperties(comp.layers[0]).entries()) {
      expect(PM.evalKfs(channel.kf, 0.5, typeof channel.v === 'string')).toBe(expected[index]![channelIndex]![0]);
      expect(PM.evalKfs(channel.kf, 0.25, typeof channel.v === 'string')).toBe(expected[index]![channelIndex]![1]);
      if (typeof channel.v === 'number') {
        expect(PM.evalKfs(channel.kf, 0.5)).toBe(45);
        expect(PM.evalKfs(channel.kf, 0.25)).toBe(22.5);
      }
      expect(channel.kf.every((key: any) => typeof key.i === 'string' && key.i.length > 0)).toBe(true);
    }
  }
  // A flag-off draw must not add temporal fields to the flag-on baseline.
  expect(JSON.stringify(project)).toBe(before);
  expect(JSON.stringify(PM.hydrateProject(JSON.parse(before)))).toBe(before);
});

it.each([
  { renamed: false, delayedMetadata: false }, { renamed: true, delayedMetadata: false },
  { renamed: false, delayedMetadata: true }
])('saves the live inactive tab after recovery failure (renamed=$renamed metadata delayed=$delayedMetadata)', async ({ renamed, delayedMetadata }) => {
  const { PM, listeners, memory } = appRegistry();
  const set = PM.store.set;
  PM.store.set = (key: string, value: any) => key === 'projectJournal.P2' ? false : set(key, JSON.parse(JSON.stringify(value)));
  PM.store.has = (key: string) => memory.has(key);
  installProjects(PM); installHistory(PM);
  // Run the real journal listener, with its storage write failing. The live
  // document and Undo remain usable even though recovery is behind them.
  PM.bus.emit = (name: string, ...args: any[]) => {
    if (name === 'history:project-patch') PM.__busHandlers.get(name)?.forEach((run: any) => run(...args));
  };
  PM.pause = vi.fn(); PM.rasterClear = vi.fn(); PM.WS.activate = vi.fn(); PM.touch = vi.fn();
  PM.assets.suspend = () => ({ map: new Map() }); PM.assets.resume = vi.fn(); PM.assets.discard = vi.fn();
  PM.replaceProject = (next: any) => { PM.proj = next; };
  PM.Projects.put(PM.proj);
  listeners.get('pm-open-project')![0]({ detail: PM.mkProject({ id: 'P2', name: 'Second' }) });
  PM.hist.do('Edit live second', () => { PM.proj.name = 'Latest second'; PM.proj.bg = '#ff0000'; });
  const live = PM.proj;
  let release!: () => void;
  const pending = new Promise<void>(resolve => { release = resolve; });
  PM.store.setSnapshotAsync = vi.fn(async () => { await pending; return false; });
  const recover = PM.Projects.recoverAsync;
  if (delayedMetadata) PM.Projects.recoverAsync = async (...args: any[]) => { await pending; return recover(...args); };
  const saveFile = vi.fn(async (_request: any) => ({ ok: true, path: '/tmp/Second.pmv' }));
  installBridgeForTests({ saveFile } as any);
  try {
    expect(await PM.Tabs.activate('P1')).toBe(true);
    expect(PM.proj.id).toBe('P1');
    if (!delayedMetadata) expect(PM.store.setSnapshotAsync).toHaveBeenCalledOnce();
    expect(PM.Projects.get('P2').name).toBe('Second');
    if (renamed) PM.Projects.rename('P2', 'Renamed from Projects');
    expect(await PM.saveProject({ projectId: 'P2' })).toBe(true);
    const saved = unpackProjectFile(saveFile.mock.calls[0]![0].data);
    expect(saved.proj.name).toBe(renamed ? 'Renamed from Projects' : 'Latest second');
    expect(saved.proj.bg).toBe('#ff0000');
    expect(saved.history.entries.map((entry: any) => entry.label)).toEqual(['Edit live second']);
    expect(PM.projectFileState('P2').dirty).toBe(false);
    expect(PM.proj.id).toBe('P1');
    expect(await PM.Tabs.activate('P2')).toBe(true);
    expect(PM.proj).toBe(live);
    expect(PM.proj.name).toBe(renamed ? 'Renamed from Projects' : 'Latest second');
    expect(PM.hist.undo()).toBe(true); expect(PM.proj.name).toBe('Second');
  } finally { PM.Projects.recoverAsync = recover; release(); }
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
  installBridgeForTests((window as any).powermove);
  expect(await PM.saveProject()).toBe(true);
  const saved = unpackProjectFile(saveFile.mock.calls[0]![0].data);
  expect(saved.history.index).toBe(0);
  expect(saved.history.entries.map((entry: any) => entry.label)).toEqual(['Rename one', 'Rename two']);
  expect(PM.Projects.getState('P1').history).toEqual(saved.history);
  const hideHome = vi.fn();
  PM.ProjectsScreen = { isOpen: true, hide: hideHome };
  (window as any).powermove.openProjectFile = async () => ({ ok: false, cancelled: true });
  await PM.openProject();
  expect(hideHome).not.toHaveBeenCalled();
  expect(PM.proj.id).toBe('P1');
  (window as any).powermove.openProjectFile = async () => ({ ok: true, path: '/tmp/Test.pmv', projectId: 'reopened', data: saveFile.mock.calls[0]![0].data });
  await PM.openProject();
  expect(hideHome).toHaveBeenCalledOnce();
  expect(toasts).not.toEqual(expect.arrayContaining([expect.stringContaining('Could not open')]));
  expect(PM.proj.id).toBe('reopened');
  expect(PM.hist.canUndo()).toBe(true);
  expect(PM.hist.canRedo()).toBe(true);
  PM.hist.redo();
  expect(PM.proj.name).toBe('Two');
  PM.hist.undo();
  expect(PM.proj.name).toBe('One');
});

it.each(['failed', 'cancelled'])('resumes outgoing asset restoration after a %s native project open without clearing live media', async outcome => {
  const { PM, toasts } = appRegistry();
  const outgoing = PM.proj;
  const runtime = { id: 'live-audio', kind: 'audio' };
  PM.sel.layers = ['selected-outgoing-layer'];
  PM.assets.map = new Map([[runtime.id, runtime]]);
  PM.assets.cancelRestores = vi.fn();
  PM.assets.clear = vi.fn();
  PM.assets.restoreProject = vi.fn(async () => ({ stale: false, missing: [] }));
  let readStarted!: () => void, rejectRead!: (error: Error) => void;
  const started = new Promise<void>(resolve => { readStarted = resolve; });
  const pending = new Promise<Uint8Array>((_resolve, reject) => { rejectRead = reject; });
  const read = vi.fn(async () => {
    readStarted();
    if (outcome === 'failed') throw new Error('Native media read failed');
    return pending;
  });
  const close = vi.fn(async () => undefined);
  PM.MediaStore = { get: async () => null, put: vi.fn(async () => true) };
  vi.stubGlobal('navigator', { storage: {} });
  installBridgeForTests({ openProjectFile: async () => ({ ok: true, path: '/tmp/Incoming.pmv',
    projectId: 'incoming', token: 'incoming-reader', size: 3,
    document: { proj: PM.mkProject({ id: 'incoming', assets: { clip: { id: 'clip' } } }) },
    media: [{ id: 'clip', type: 'audio/wav', offset: 0, length: 3 }] }), projectRead: { read, close } } as any);
  const opening = PM.openProject();
  await started;
  if (outcome === 'cancelled') {
    expect(await PM.openProjectHere(outgoing.id)).toBe(true);
    rejectRead(new Error('Superseded read finished'));
  }
  await opening;
  await PM.projectAssetsReady;
  expect(PM.assets.cancelRestores).toHaveBeenCalledOnce();
  expect(PM.assets.cancelRestores.mock.invocationCallOrder[0]).toBeLessThan(read.mock.invocationCallOrder[0]!);
  expect(PM.assets.restoreProject).toHaveBeenCalledOnce();
  expect(PM.assets.restoreProject).toHaveBeenCalledWith(outgoing);
  expect(PM.assets.clear).not.toHaveBeenCalled();
  expect(PM.proj).toBe(outgoing);
  expect(PM.sel.layers).toEqual(['selected-outgoing-layer']);
  expect(PM.assets.map.get(runtime.id)).toBe(runtime);
  expect(PM.projectLoading).toBeNull();
  expect(close).toHaveBeenCalledWith('incoming-reader');
  expect(PM.MediaStore.put).not.toHaveBeenCalled();
  expect(toasts.some(message => message.startsWith('Could not open'))).toBe(outcome === 'failed');
});

it('opens streamed document ranges with Unicode and working Undo and Redo', async () => {
  const { PM, toasts } = appRegistry();
  installHistory(PM);
  PM.pause = vi.fn(); PM.rasterClear = vi.fn(); PM.WS.activate = vi.fn(); PM.touch = vi.fn();
  PM.replaceProject = (next: any) => { PM.proj = next; };
  PM.proj.note = 'x'.repeat(1024 * 1024) + '🟢 Café';
  PM.hist.do('Rename ☃ one', () => { PM.proj.name = 'One ☃'; });
  PM.hist.do('Rename 🟢 two', () => { PM.proj.name = 'Two 🟢'; });
  PM.hist.undo();
  const document = { proj: PM.proj, history: PM.hist.export({ copy: false }) };
  const bytes = new TextEncoder().encode(JSON.stringify(document)), offset = 5;
  const read = vi.fn(async (token: string, at: number, length: number) => {
    expect(token).toBe('streamed-document');
    return bytes.subarray(at - offset, at - offset + length);
  });
  const close = vi.fn(async () => undefined);
  PM.MediaStore = { get: async () => null, put: async () => true };
  (window as any).powermove = { openProjectFile: async () => ({ ok: true, path: '/tmp/Streamed.pmv',
    projectId: 'streamed-open', token: 'streamed-document', size: offset + bytes.length, media: [],
    documentRange: { offset, length: bytes.length, sha256: sha256Hex(bytes) } }), projectRead: { read, close } };
  installBridgeForTests((window as any).powermove);
  await PM.openProject();
  expect(PM.proj.id).toBe('streamed-open');
  expect(PM.proj.note).toBe(document.proj.note);
  expect(PM.hist.export().entries.map((entry: any) => entry.label)).toEqual(['Rename ☃ one', 'Rename 🟢 two']);
  expect(PM.hist.canUndo()).toBe(true); expect(PM.hist.canRedo()).toBe(true);
  PM.hist.redo(); expect(PM.proj.name).toBe('Two 🟢');
  PM.hist.undo(); expect(PM.proj.name).toBe('One ☃');
  expect(read.mock.calls.length).toBeGreaterThan(1);
  expect(read.mock.calls.every(([, , length]) => length <= 1024 * 1024)).toBe(true);
  expect(close).toHaveBeenCalledWith('streamed-document');
  expect(toasts.some(message => message.startsWith('Could not open'))).toBe(false);
});

it('does not recapture a trashed active project while switching, but still captures an ordinary outgoing project', () => {
  for (const trashed of [true, false]) {
    const { PM, listeners } = appRegistry();
    const put = vi.spyOn(PM.Projects, 'put');
    PM.Projects.trashList = () => trashed ? [{ id: 'P1' }] : [];
    PM.pause = vi.fn();
    PM.rasterClear = vi.fn();
    PM.WS.activate = vi.fn();
    PM.touch = vi.fn();
    const next = PM.mkProject({ id: 'P2', name: 'Next' });

    listeners.get('pm-open-project')![0]({ detail: next });

    expect(PM.proj.id).toBe('P2');
    expect(put.mock.calls.map(([project]) => (project as { id: string }).id)).toEqual(trashed ? ['P2'] : ['P1', 'P2']);
  }
});

describe('minReaderVersion guard', () => {
  const NEWER = 'This project needs a newer version of Powermove';
  function editorWith(bridge: any, bootProject?: any) {
    const registry = appRegistry(true, bootProject, undefined, bridge);
    const { PM } = registry;
    PM.pause = vi.fn();
    PM.rasterClear = vi.fn();
    PM.WS.activate = vi.fn();
    PM.touch = vi.fn();
    PM.dismissToast = vi.fn();
    PM.hist.export = () => ({ version: 1, index: 0, entries: [{ label: 'edit' }] });
    return registry;
  }
  const fileBytes = (document: any) => new TextEncoder().encode(JSON.stringify(document));
  const newerDocument = () => ({
    v: '9.0.0',
    proj: { id: 'written-by-newer', name: 'Newer', w: 1920, h: 1080, fps: 30, dur: 10, layers: [], assets: {}, markers: [], comps: {}, minReaderVersion: 2 },
  });

  it('opens a newer project read-only: notice, no autosave, no Save or Save As', async () => {
    const saveFile = vi.fn(async () => ({ ok: true, path: '/tmp/Newer.pmv' }));
    const projectSave = { begin: vi.fn(), chunk: vi.fn(), finish: vi.fn(), abort: vi.fn() };
    const confirmProjectClose = vi.fn(async () => 'save');
    const bridge: any = { saveFile, projectSave, confirmProjectClose };
    const { PM, toasts, timers } = editorWith(bridge);
    bridge.openProjectFile = async () => ({ ok: true, path: '/tmp/Newer.pmv', projectId: 'opened-newer', data: fileBytes(newerDocument()) });
    await PM.openProject();

    expect(PM.proj.id).toBe('opened-newer');
    expect(PM.proj.minReaderVersion).toBe(2);
    expect(PM.isReadOnlyProject()).toBe(true);
    expect(toasts).toContain(`${NEWER}. It is open read-only, so changes are not saved.`);
    expect(toasts).toContain('Opened Newer.pmv');
    expect(PM.Projects.getState('opened-newer').history).toBeUndefined();

    const put = vi.spyOn(PM.Projects, 'put');
    const putState = vi.spyOn(PM.Projects, 'putState');
    PM.proj.name = 'Edited in an older build';
    PM.autosave();
    await timers.get(PM.app.saveTimer)();
    expect(put).not.toHaveBeenCalled();
    expect(putState).not.toHaveBeenCalled();
    expect(PM.app.dirty).toBe(true);

    expect(await PM.saveProject()).toBe(false);
    expect(await PM.saveProject({ saveAs: true })).toBe(false);
    expect(await PM.saveProject({ projectId: 'opened-newer' })).toBe(false);
    expect(toasts.filter(text => text === `${NEWER}. It is open read-only, so it can't be saved.`)).toHaveLength(3);
    expect(saveFile).not.toHaveBeenCalled();
    expect(projectSave.begin).not.toHaveBeenCalled();

    // Choosing Save when closing cannot write either; Don't Save closes.
    expect(await PM.confirmCloseProject('opened-newer')).toBe(false);
    expect(saveFile).not.toHaveBeenCalled();
    confirmProjectClose.mockResolvedValueOnce('discard');
    expect(await PM.confirmCloseProject('opened-newer')).toBe(true);

    // Session capture keeps the layout but records no undo history.
    PM.flushProject = PM.flushProject || (() => undefined);
    await PM.flushProject();
    expect(putState.mock.calls.every(([, state]: any) => !('history' in state))).toBe(true);
  });

  it('carries a stamp on the file envelope onto the project', async () => {
    const bridge: any = {};
    const { PM } = editorWith(bridge);
    const document: any = newerDocument();
    delete document.proj.minReaderVersion;
    document.minReaderVersion = 5;
    bridge.openProjectFile = async () => ({ ok: true, path: '/tmp/Envelope.pmv', projectId: 'envelope', data: fileBytes(document) });
    await PM.openProject();
    expect(PM.proj.id).toBe('envelope');
    expect(PM.proj.minReaderVersion).toBe(5);
    expect(PM.isReadOnlyProject('envelope')).toBe(true);
  });

  it('explains why a newer project it cannot parse does not open', async () => {
    const bridge: any = {};
    const { PM, toasts } = editorWith(bridge);
    bridge.openProjectFile = async () => ({ ok: true, path: '/tmp/Reshaped.pmv', projectId: 'reshaped', data: fileBytes({ proj: { minReaderVersion: 3, timelines: [] } }) });
    await PM.openProject();
    expect(PM.proj.id).toBe('P1');
    expect(toasts).toContain(`Could not open project: ${NEWER}.`);
  });

  it('shows the notice and skips autosave for a newer project restored at launch', async () => {
    const { PM, toasts, timers } = editorWith({}, { ...newerDocument().proj, id: 'P1' });
    expect(PM.proj.minReaderVersion).toBe(2);
    expect(toasts).toContain(`${NEWER}. It is open read-only, so changes are not saved.`);
    const put = vi.spyOn(PM.Projects, 'put');
    PM.autosave();
    await timers.get(PM.app.saveTimer)();
    expect(put).not.toHaveBeenCalled();
  });

  it('keeps saving projects stamped for this reader', async () => {
    const saveFile = vi.fn(async (_request: any) => ({ ok: true, path: '/tmp/Current.pmv' }));
    const bridge: any = { saveFile };
    const { PM, toasts } = editorWith(bridge);
    const document: any = newerDocument();
    document.proj.minReaderVersion = 1;
    bridge.openProjectFile = async () => ({ ok: true, path: '/tmp/Current.pmv', projectId: 'current', data: fileBytes(document) });
    await PM.openProject();
    expect(PM.isReadOnlyProject()).toBe(false);
    expect(toasts.some(text => text.startsWith(NEWER))).toBe(false);
    expect(await PM.saveProject()).toBe(true);
    expect(unpackProjectFile(saveFile.mock.calls[0]![0].data).proj.minReaderVersion).toBe(1);
  });
});
