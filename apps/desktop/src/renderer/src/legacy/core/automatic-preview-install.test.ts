import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { AutomaticPreviewOptions } from './automatic-preview';
import { installAutomaticPreview } from './automatic-preview-install';

const factories = vi.hoisted(() => ({ automatic: vi.fn(), videos: vi.fn(), viewer: vi.fn() }));
vi.mock('./automatic-preview', () => ({ createAutomaticPreview: factories.automatic }));
vi.mock('./automatic-preview-video', () => ({ createAutomaticPreviewVideos: factories.videos }));
vi.mock('./services', () => ({ viewerService: factories.viewer }));

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

function editor() {
  let clock = 0, budget = 128 * 1024 * 1024, version = 1, staging = 0;
  const listeners = new Map<string, Set<() => void>>();
  const windowTarget = Object.assign(new EventTarget(), {
    document: Object.assign(new EventTarget(), { hidden: false, fonts: { status: 'loaded' } }),
    requestIdleCallback: undefined as undefined | ((callback: IdleRequestCallback, options?: IdleRequestOptions) => number),
    cancelIdleCallback: vi.fn(),
  });
  const documentTarget = windowTarget.document;
  const inputPending = vi.fn(() => false);
  vi.stubGlobal('window', windowTarget);
  vi.stubGlobal('document', documentTarget);
  vi.stubGlobal('navigator', { scheduling: { isInputPending: inputPending } });
  vi.stubGlobal('performance', { now: () => clock });
  const automatic = { request: vi.fn(), cancel: vi.fn(), dispose: vi.fn() };
  const frames = new Map<string, ImageBitmap>();
  const videos = {
    prepare: vi.fn(async (_time: number, _valid: () => boolean): Promise<Map<string, ImageBitmap> | null> => frames),
    release: vi.fn(), cancel: vi.fn(), dispose: vi.fn(), get bytes() { return staging; },
  };
  const viewer = { preview: { activeId: null as string | null } };
  const PM: any = {
    proj: { layers: [{ id: 'shape', type: 'shape' }], fps: 24, dur: 10, work: [1, 4], shutter: .75 },
    time: 2, playing: false, previewFps: 0,
    Export: { busy: false }, Preview: { preparing: false, active: false, bytes: 32 },
    ProjectsScreen: { isOpen: false }, SettingsUI: { isOpen: false },
    assets: { loading: new Set() }, interactionActive: vi.fn(() => false), animVersion: () => version,
    GL: {
      gl: {}, canvas: { width: 960, height: 540 }, previewFrames: { bytes: 64 },
      compiling: 0, errors: new Map(), previewPreparationKey: vi.fn(() => 'render-signature'),
      hasPreviewFrame: vi.fn(() => true), preparePreviewFrame: vi.fn(async () => true),
      cancelPreviewPreparation: vi.fn(), reservePreviewPreparation: vi.fn(),
    },
    Memory: { budget: vi.fn(() => budget) },
    bus: {
      on: vi.fn((event: string, callback: () => void) => {
        let bucket = listeners.get(event);
        if (!bucket) listeners.set(event, bucket = new Set());
        bucket.add(callback);
        return vi.fn(() => { bucket.delete(callback); });
      }),
      emit: (event: string) => { for (const callback of listeners.get(event) || []) callback(); },
    },
  };
  PM.ProjectIndex = { allLayers: vi.fn(() => PM.proj.layers) };
  factories.automatic.mockReturnValue(automatic);
  factories.videos.mockReturnValue(videos);
  factories.viewer.mockReturnValue(viewer);
  const service = installAutomaticPreview(PM);
  const options = factories.automatic.mock.calls.at(-1)![0] as AutomaticPreviewOptions;
  return {
    PM, service, options, automatic, videos, frames, viewer, windowTarget, documentTarget,
    inputPending, listeners,
    setClock: (time: number) => { clock = time; },
    setBudget: (bytes: number) => { budget = bytes; },
    setVersion: (next: number) => { version = next; },
    setStaging: (bytes: number) => { staging = bytes; },
  };
}

beforeEach(() => { vi.clearAllMocks(); vi.useFakeTimers(); });
afterEach(() => { vi.useRealTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });

describe('automatic preview editor integration', () => {
  it('shares the existing preview budget and exposes the actual viewport, work area and render signature', () => {
    const h = editor();
    expect(h.PM.AutomaticPreview).toBe(h.service);
    expect(factories.videos).toHaveBeenCalledExactlyOnceWith(h.PM);
    expect(h.options.state()).toEqual({
      project: h.PM.proj, key: 'render-signature', time: 2, fps: 24,
      start: 1, end: 4, blocked: false, budget: 128 * 1024 * 1024,
      bytes: 96, frameBytes: 960 * 540 * 4,
    });
    expect(h.PM.Memory.budget).toHaveBeenCalledWith('preview');
    h.PM.previewFps = 12;
    h.PM.proj.work = [4, 4];
    expect(h.options.state()).toMatchObject({ fps: 12, start: 0, end: 10 });
  });

  it.each([
    ['playback', (h: ReturnType<typeof editor>) => { h.PM.playing = true; }],
    ['export', (h: ReturnType<typeof editor>) => { h.PM.Export.busy = true; }],
    ['agent capture', (h: ReturnType<typeof editor>) => { h.PM.agentFrameCapture = true; }],
    ['manual preview preparation', (h: ReturnType<typeof editor>) => { h.PM.Preview.preparing = true; }],
    ['manual preview playback', (h: ReturnType<typeof editor>) => { h.PM.Preview.active = true; }],
    ['prepared foreground video', (h: ReturnType<typeof editor>) => { h.PM.preparedVideoFrames = new Map(); }],
    ['loading assets', (h: ReturnType<typeof editor>) => { h.PM.assets.loading.add('clip'); }],
    ['shader compilation', (h: ReturnType<typeof editor>) => { h.PM.GL.compiling = 1; }],
    ['renderer errors', (h: ReturnType<typeof editor>) => { h.PM.GL.errors.set('shader', 'error'); }],
    ['editor input', (h: ReturnType<typeof editor>) => { h.PM.interactionActive.mockReturnValue(true); }],
    ['pending input', (h: ReturnType<typeof editor>) => { h.inputPending.mockReturnValue(true); }],
    ['Home', (h: ReturnType<typeof editor>) => { h.PM.ProjectsScreen.isOpen = true; }],
    ['Settings', (h: ReturnType<typeof editor>) => { h.PM.SettingsUI.isOpen = true; }],
    ['source viewer', (h: ReturnType<typeof editor>) => { h.viewer.preview.activeId = 'source-clip'; }],
    ['hidden editor', (h: ReturnType<typeof editor>) => { h.documentTarget.hidden = true; }],
    ['loading fonts', (h: ReturnType<typeof editor>) => { h.documentTarget.fonts.status = 'loading'; }],
    ['disabled preview memory', (h: ReturnType<typeof editor>) => { h.setBudget(0); }],
    ['missing GL context', (h: ReturnType<typeof editor>) => { h.PM.GL.gl = null; }],
    ['empty viewport', (h: ReturnType<typeof editor>) => { h.PM.GL.canvas.width = 0; }],
    ['empty project', (h: ReturnType<typeof editor>) => { h.PM.proj.layers = []; }],
  ])('blocks idle preparation during %s and cancels private work once on transition', (_name, block) => {
    const h = editor();
    h.service.request();
    expect(h.options.state().blocked).toBe(false);
    block(h);
    expect(h.options.state().blocked).toBe(true);
    h.service.request();
    expect(h.PM.GL.cancelPreviewPreparation).toHaveBeenCalledTimes(1);
    expect(h.videos.cancel).toHaveBeenCalledTimes(1);
    for (let request = 0; request < 100; request++) h.service.request();
    expect(h.PM.GL.cancelPreviewPreparation).toHaveBeenCalledTimes(1);
    expect(h.videos.cancel).toHaveBeenCalledTimes(1);
  });

  it('uses playback-quality effect options for lookup and preparation and releases private frames afterward', async () => {
    const h = editor();
    const expectedOptions = { mblur: true, mbSamples: 6, shutter: .75, hideShy: false };
    expect(h.options.has(5 / 24)).toBe(true);
    expect(h.PM.GL.hasPreviewFrame).toHaveBeenCalledExactlyOnceWith(5 / 24, expectedOptions);
    const valid = vi.fn(() => true);
    expect(await h.options.prepare(5 / 24, valid)).toBe(true);
    expect(h.videos.prepare).toHaveBeenCalledExactlyOnceWith(5 / 24, valid);
    expect(h.PM.GL.preparePreviewFrame).toHaveBeenCalledExactlyOnceWith(5 / 24, expectedOptions, h.frames, valid);
    expect(h.videos.release).toHaveBeenCalledExactlyOnceWith(h.frames);
    expect(h.PM.time).toBe(2);
    expect(h.PM.playing).toBe(false);
    expect(h.PM.preparedVideoFrames).toBeUndefined();
  });

  it('does not decode or submit a frame which was already invalidated', async () => {
    const h = editor();
    expect(await h.options.prepare(1, () => false)).toBe(false);
    expect(h.videos.prepare).not.toHaveBeenCalled();
    expect(h.PM.GL.preparePreviewFrame).not.toHaveBeenCalled();
    expect(h.videos.release).not.toHaveBeenCalled();
  });

  it('releases a decoded frame canceled before the GPU render starts', async () => {
    const h = editor();
    const waiting = deferred<Map<string, ImageBitmap> | null>();
    let current = true;
    h.videos.prepare.mockReturnValue(waiting.promise);
    const preparation = h.options.prepare(1, () => current);
    current = false;
    waiting.resolve(h.frames);
    expect(await preparation).toBe(false);
    expect(h.PM.GL.preparePreviewFrame).not.toHaveBeenCalled();
    expect(h.videos.release).toHaveBeenCalledExactlyOnceWith(h.frames);
  });

  it('does not render when private video preparation declines unsafe or oversized media', async () => {
    const h = editor();
    h.videos.prepare.mockResolvedValue(null);
    expect(await h.options.prepare(1, () => true)).toBe(false);
    expect(h.PM.GL.preparePreviewFrame).not.toHaveBeenCalled();
    expect(h.videos.release).not.toHaveBeenCalled();
  });

  it('retains private frames until the GPU fence finishes, then releases them even when rendering fails', async () => {
    const h = editor();
    const gpu = deferred<boolean>();
    h.PM.GL.preparePreviewFrame.mockReturnValue(gpu.promise);
    const preparation = h.options.prepare(1, () => true);
    await Promise.resolve();
    expect(h.PM.GL.preparePreviewFrame).toHaveBeenCalledTimes(1);
    expect(h.videos.release).not.toHaveBeenCalled();
    gpu.reject(new Error('Context lost'));
    await expect(preparation).rejects.toThrow('Context lost');
    expect(h.videos.release).toHaveBeenCalledExactlyOnceWith(h.frames);
  });

  it('blocks temporal video motion blur but allows ordinary video and isolated shape motion blur', () => {
    const h = editor();
    h.PM.proj.layers = [{ id: 'video', type: 'video' }];
    h.setVersion(2);
    expect(h.options.state().blocked).toBe(false);
    h.PM.proj.layers.push({ id: 'blurred-shape', type: 'shape', mblur: true });
    h.setVersion(3);
    expect(h.options.state().blocked).toBe(true);
    h.PM.proj.layers = [{ id: 'blurred-shape', type: 'shape', mblur: true }];
    h.setVersion(4);
    expect(h.options.state().blocked).toBe(false);
  });

  it('cancels ongoing work immediately when an edit makes temporal video preparation unsafe', () => {
    const h = editor();
    h.service.request();
    h.options.state();
    h.PM.proj.layers = [{ type: 'shape', mblur: true }, { type: 'video' }];
    h.setVersion(2);
    h.PM.bus.emit('layers');
    expect(h.options.state().blocked).toBe(true);
    expect(h.PM.GL.cancelPreviewPreparation).toHaveBeenCalledTimes(1);
    expect(h.videos.cancel).toHaveBeenCalledTimes(1);
  });

  it('checks nested video eligibility once per project revision, including a project switch with the same revision', () => {
    const h = editor();
    h.PM.ProjectIndex.allLayers.mockReturnValue([{ type: 'shape', mblur: true }, { type: 'video' }]);
    expect(h.options.state().blocked).toBe(true);
    for (let call = 0; call < 100; call++) h.options.state();
    expect(h.PM.ProjectIndex.allLayers).toHaveBeenCalledTimes(1);
    h.PM.proj = { ...h.PM.proj, layers: [{ type: 'shape' }] };
    h.PM.ProjectIndex.allLayers.mockReturnValue(h.PM.proj.layers);
    expect(h.options.state().blocked).toBe(false);
    expect(h.PM.ProjectIndex.allLayers).toHaveBeenCalledTimes(2);
  });

  it('releases GPU and private decoder work on memory pressure and stays idle for the cooldown', () => {
    const h = editor();
    h.service.request();
    h.windowTarget.dispatchEvent(new Event('memorypressure'));
    expect(h.automatic.cancel).toHaveBeenCalledTimes(1);
    expect(h.PM.GL.cancelPreviewPreparation).toHaveBeenCalledTimes(1);
    expect(h.videos.cancel).toHaveBeenCalledTimes(1);
    expect(h.options.state().blocked).toBe(true);
    h.setClock(29_999);
    expect(h.options.state().blocked).toBe(true);
    h.setClock(30_000);
    expect(h.options.state().blocked).toBe(false);
    h.service.request();
    expect(h.automatic.request).toHaveBeenCalledTimes(2);
  });

  it('reports temporary decoded-image bytes live to the shared memory manager', () => {
    const h = editor();
    expect(h.service.stagingBytes).toBe(0);
    h.setStaging(12 * 1024 * 1024);
    expect(h.PM.AutomaticPreview.stagingBytes).toBe(12 * 1024 * 1024);
    h.setStaging(0);
    expect(h.service.stagingBytes).toBe(0);
  });

  it('makes room for a new work area once without raising the shared RAM limit', () => {
    const h = editor();
    h.setBudget(256);
    h.PM.GL.canvas = { width: 4, height: 4 };
    h.PM.Preview.bytes = 0;
    h.PM.GL.previewFrames.bytes = 256;
    h.PM.GL.hasPreviewFrame.mockReturnValue(false);
    h.PM.GL.reservePreviewPreparation.mockImplementation(() => { h.PM.GL.previewFrames.bytes = 0; });
    h.service.request();
    expect(h.PM.GL.reservePreviewPreparation).toHaveBeenCalledExactlyOnceWith(256);
    expect(h.PM.GL.hasPreviewFrame).toHaveBeenCalledExactlyOnceWith(2, {
      mblur: true, mbSamples: 6, shutter: .75, hideShy: false,
    });
    expect(h.options.state()).toMatchObject({ budget: 256, bytes: 0, frameBytes: 64 });
    // Other foreground frames can refill the cache; the same idle area must
    // not keep deleting that useful work on every animation frame request.
    for (let request = 0; request < 100; request++) {
      h.PM.GL.previewFrames.bytes = 256;
      h.PM.time = 2.001;
      h.service.request();
    }
    expect(h.PM.GL.reservePreviewPreparation).toHaveBeenCalledTimes(1);
    expect(h.PM.GL.hasPreviewFrame).toHaveBeenCalledTimes(1);
    h.PM.time = 3;
    h.service.request();
    expect(h.PM.GL.reservePreviewPreparation).toHaveBeenCalledTimes(2);
    expect(h.options.state()).toMatchObject({ budget: 256, bytes: 0 });
  });

  it.each([[20, 432], [1000, 4096]])('reserves the remaining %i-second work area within the RAM and frame-count limits', (end, frames) => {
    const h = editor();
    h.setBudget(1024 * 1024);
    h.PM.GL.canvas = { width: 4, height: 4 };
    h.PM.Preview.bytes = 0;
    h.PM.GL.previewFrames.bytes = 1024 * 1024;
    h.PM.proj.work = [0, end];
    h.PM.GL.hasPreviewFrame.mockReturnValue(false);
    h.service.request();
    expect(h.PM.GL.reservePreviewPreparation).toHaveBeenCalledExactlyOnceWith(frames * 64);
  });

  it('makes room for a later missing frame even when the first pictures are already ready', () => {
    const h = editor();
    h.setBudget(256); h.PM.GL.canvas = { width: 4, height: 4 };
    h.PM.Preview.bytes = 0; h.PM.GL.previewFrames.bytes = 256;
    h.PM.GL.hasPreviewFrame.mockImplementation((time: number) => time < 2 + 2 / 24);
    h.service.request();
    expect(h.PM.GL.hasPreviewFrame).toHaveBeenCalledTimes(3);
    expect(h.PM.GL.reservePreviewPreparation).toHaveBeenCalledExactlyOnceWith(256);
    h.service.request(); expect(h.PM.GL.reservePreviewPreparation).toHaveBeenCalledTimes(1);
  });

  it('preserves a full cache while editing and makes room only after input stops', () => {
    const h = editor();
    h.setBudget(256);
    h.PM.GL.canvas = { width: 4, height: 4 };
    h.PM.Preview.bytes = 0;
    h.PM.GL.previewFrames.bytes = 256;
    h.PM.GL.hasPreviewFrame.mockReturnValue(false);
    h.PM.interactionActive.mockReturnValue(true);
    h.service.request();
    expect(h.PM.GL.reservePreviewPreparation).not.toHaveBeenCalled();
    expect(h.PM.GL.previewFrames.bytes).toBe(256);
    h.PM.interactionActive.mockReturnValue(false);
    h.service.request();
    expect(h.PM.GL.reservePreviewPreparation).toHaveBeenCalledTimes(1);
  });

  it('preserves a full cache when the whole upcoming range is already prepared', () => {
    const h = editor();
    h.setBudget(256);
    h.PM.GL.canvas = { width: 4, height: 4 };
    h.PM.Preview.bytes = 0;
    h.PM.GL.previewFrames.bytes = 256;
    h.service.request();
    expect(h.PM.GL.hasPreviewFrame).toHaveBeenCalledTimes(4);
    expect(h.PM.GL.reservePreviewPreparation).not.toHaveBeenCalled();
    expect(h.PM.GL.previewFrames.bytes).toBe(256);
  });

  it.each([[.051, .179, 3], [.051, .079, 0]])('preserves ready frames for fractional work area %f–%f', (start, end, count) => {
    const h = editor();
    h.setBudget(256); h.PM.GL.canvas = { width: 4, height: 4 };
    h.PM.Preview.bytes = 0; h.PM.GL.previewFrames.bytes = 256;
    h.PM.time = 0; h.PM.proj.work = [start, end];
    h.PM.GL.hasPreviewFrame.mockImplementation((time: number) => time >= start && time < end);
    h.service.request();
    expect(h.PM.GL.hasPreviewFrame).toHaveBeenCalledTimes(count);
    expect(h.PM.GL.reservePreviewPreparation).not.toHaveBeenCalled();
    expect(h.PM.GL.previewFrames.bytes).toBe(256);
  });

  it('does not trim existing frames when there is already space for idle preparation', () => {
    const h = editor();
    h.PM.GL.hasPreviewFrame.mockReturnValue(false);
    h.service.request();
    expect(h.PM.GL.hasPreviewFrame).not.toHaveBeenCalled();
    expect(h.PM.GL.reservePreviewPreparation).not.toHaveBeenCalled();
    expect(h.options.state().bytes).toBe(96);
  });

  it.each([
    ['hidden', (h: ReturnType<typeof editor>) => { h.documentTarget.hidden = true; }],
    ['Home', (h: ReturnType<typeof editor>) => { h.PM.ProjectsScreen.isOpen = true; }],
    ['playing', (h: ReturnType<typeof editor>) => { h.PM.playing = true; }],
  ])('does not build render signatures or scan projects while continuously %s', (_name, block) => {
    const h = editor();
    block(h);
    for (let request = 0; request < 100; request++) h.service.request();
    expect(h.PM.GL.previewPreparationKey).not.toHaveBeenCalled();
    expect(h.PM.ProjectIndex.allLayers).not.toHaveBeenCalled();
    expect(h.PM.GL.hasPreviewFrame).not.toHaveBeenCalled();
    expect(h.PM.GL.reservePreviewPreparation).not.toHaveBeenCalled();
    expect(h.automatic.request).not.toHaveBeenCalled();
  });

  it('reacts to transport, edits, navigation and visibility without owning those controls', () => {
    const h = editor();
    for (const event of ['transport', 'time', 'project', 'layers', 'assets', 'quality', 'fonts', 'projects:screen', 'settings:screen']) {
      const before = h.automatic.request.mock.calls.length;
      h.PM.bus.emit(event);
      expect(h.automatic.request).toHaveBeenCalledTimes(before + 1);
    }
    h.documentTarget.hidden = true;
    h.documentTarget.dispatchEvent(new Event('visibilitychange'));
    expect(h.PM.GL.cancelPreviewPreparation).toHaveBeenCalledTimes(1);
    expect(h.videos.cancel).toHaveBeenCalledTimes(1);
    expect(h.options.state().blocked).toBe(true);
  });

  it('dispose removes every subscription, releases resources once and ignores later editor events', () => {
    const h = editor();
    h.service.request();
    h.service.dispose();
    h.service.dispose();
    expect(h.automatic.dispose).toHaveBeenCalledTimes(1);
    expect(h.PM.GL.cancelPreviewPreparation).toHaveBeenCalledTimes(1);
    expect(h.videos.dispose).toHaveBeenCalledTimes(1);
    expect([...h.listeners.values()].every(listeners => listeners.size === 0)).toBe(true);
    for (const event of h.listeners.keys()) h.PM.bus.emit(event);
    h.documentTarget.dispatchEvent(new Event('visibilitychange'));
    h.windowTarget.dispatchEvent(new Event('memorypressure'));
    h.windowTarget.dispatchEvent(new Event('pagehide'));
    h.service.request();
    expect(h.automatic.request).toHaveBeenCalledTimes(1);
    expect(h.automatic.cancel).not.toHaveBeenCalled();
    expect(h.automatic.dispose).toHaveBeenCalledTimes(1);
  });

  it('pagehide disposes the service and its listeners', () => {
    const h = editor();
    h.windowTarget.dispatchEvent(new Event('pagehide'));
    expect(h.automatic.dispose).toHaveBeenCalledTimes(1);
    expect(h.videos.dispose).toHaveBeenCalledTimes(1);
    expect([...h.listeners.values()].every(listeners => listeners.size === 0)).toBe(true);
  });

  it('defers short idle deadlines without canceling preparation or restarting its idle wait', () => {
    const h = editor();
    const callbacks: IdleRequestCallback[] = [];
    h.windowTarget.requestIdleCallback = vi.fn(fn => { callbacks.push(fn); return callbacks.length; });
    const run = vi.fn();
    h.options.schedule(run, 100);
    vi.advanceTimersByTime(99);
    expect(h.windowTarget.requestIdleCallback).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(run).not.toHaveBeenCalled();
    expect(h.windowTarget.requestIdleCallback).toHaveBeenCalledWith(expect.any(Function), { timeout: 1000 });
    callbacks[0]!({ didTimeout: false, timeRemaining: () => 0 });
    expect(run).not.toHaveBeenCalled();
    expect(h.windowTarget.requestIdleCallback).toHaveBeenCalledTimes(2);
    callbacks[1]!({ didTimeout: false, timeRemaining: () => 1 });
    expect(h.windowTarget.requestIdleCallback).toHaveBeenCalledTimes(3);
    expect(h.automatic.cancel).not.toHaveBeenCalled();
    expect(run).not.toHaveBeenCalled();
    callbacks[2]!({ didTimeout: false, timeRemaining: () => 2 });
    expect(run).toHaveBeenCalledTimes(1);
    expect(h.windowTarget.requestIdleCallback).toHaveBeenCalledTimes(3);
  });

  it('makes progress at the idle timeout even when the browser never offers a long enough slice', () => {
    const h = editor();
    const callbacks: IdleRequestCallback[] = [];
    h.windowTarget.requestIdleCallback = vi.fn(fn => { callbacks.push(fn); return callbacks.length; });
    const run = vi.fn();
    const cancel = h.options.schedule(run, 100);
    vi.advanceTimersByTime(100);
    callbacks[0]!({ didTimeout: false, timeRemaining: () => 1 });
    expect(run).not.toHaveBeenCalled();
    callbacks[1]!({ didTimeout: true, timeRemaining: () => 0 });
    expect(run).toHaveBeenCalledTimes(1);
    expect(h.automatic.cancel).not.toHaveBeenCalled();
    cancel();
    expect(h.windowTarget.cancelIdleCallback).not.toHaveBeenCalled();
  });

  it('cancels the current deferred idle handle and ignores late callbacks from any earlier attempt', () => {
    const h = editor();
    const callbacks: IdleRequestCallback[] = [];
    h.windowTarget.requestIdleCallback = vi.fn(fn => { callbacks.push(fn); return callbacks.length; });
    const run = vi.fn();
    const cancel = h.options.schedule(run, 100);
    vi.advanceTimersByTime(100);
    callbacks[0]!({ didTimeout: false, timeRemaining: () => 0 });
    callbacks[1]!({ didTimeout: false, timeRemaining: () => 1 });
    cancel();
    expect(h.windowTarget.cancelIdleCallback).toHaveBeenCalledExactlyOnceWith(3);
    callbacks[2]!({ didTimeout: true, timeRemaining: () => 0 });
    callbacks[0]!({ didTimeout: false, timeRemaining: () => 20 });
    expect(run).not.toHaveBeenCalled();
    expect(h.windowTarget.requestIdleCallback).toHaveBeenCalledTimes(3);
  });

  it('cancels both delayed timers and scheduled browser idle callbacks', () => {
    const h = editor();
    h.windowTarget.requestIdleCallback = vi.fn(() => 9);
    const run = vi.fn();
    const cancelTimer = h.options.schedule(run, 100);
    cancelTimer();
    vi.advanceTimersByTime(100);
    expect(h.windowTarget.requestIdleCallback).not.toHaveBeenCalled();
    const cancelIdle = h.options.schedule(run, 100);
    vi.advanceTimersByTime(100);
    expect(h.windowTarget.requestIdleCallback).toHaveBeenCalledTimes(1);
    cancelIdle();
    expect(h.windowTarget.cancelIdleCallback).toHaveBeenCalledExactlyOnceWith(9);
    expect(run).not.toHaveBeenCalled();
  });

  it('can schedule preparation without requestIdleCallback on older supported environments', () => {
    const h = editor();
    const run = vi.fn();
    h.options.schedule(run, 100);
    vi.advanceTimersByTime(99);
    expect(run).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(run).toHaveBeenCalledTimes(1);
  });
});
