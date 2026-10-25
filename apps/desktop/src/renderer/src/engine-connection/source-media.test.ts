// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { EngineSourceMedia, sourcePosterSize } from './source-media';
import type { EngineSnapshot, EngineRenderTransport } from './render-transport';
import type { EngineBridge } from '../../../shared/engine-ipc';
import type { DocumentSession } from './session';
import { ticks } from './transport';
import { SourceVideoPresenter } from './source-video-presenter';

function fixture() {
  const project = { id: 'project', assets: { clip: { id: 'clip', kind: 'video', w: 1920, h: 1080, dur: 20, contentHash: 'original', rendition: false } } };
  const pixels = { width: 4, height: 2, data: new Uint8ClampedArray(32), revision: 1, pixels: 'straight' } as EngineSnapshot;
  const snapshot = vi.fn(async () => pixels);
  const encoded: string[] = [];
  const toCanvas = vi.fn(() => ({ toBlob: (answer: (blob: Blob) => void, type: string) => { encoded.push(type); answer(new Blob(['poster'], { type })); } }));
  const rendition = vi.fn(async () => ({ data: new Uint8Array([1, 2]), type: 'video/webm' as const }));
  const strip = vi.fn(async (_request: unknown): Promise<{ width: number; height: number; count: number; data: Uint8Array }> => ({ width: 4, height: 2, count: 2, data: new Uint8Array(64) }));
  const prepare = vi.fn(async (_request: unknown) => {}), release = vi.fn(async () => {});
  const cancel = vi.fn(async () => {}), audio = vi.fn();
  const bind = vi.fn(async () => {});
  const changed = vi.fn(), error = vi.fn();
  const source = new EngineSourceMedia({ project: () => project, revision: () => 7,
    render: { snapshot, toCanvas } as unknown as EngineRenderTransport,
    native: { browserRendition: rendition, sourceFilmstrip: strip, prepareSource: prepare, releasePreparedSources: release, cancelMediaJob: cancel } as unknown as EngineBridge,
    session: () => ({ audio } as unknown as DocumentSession), wake: async () => {}, bind,
    changed, error
  });
  return { project, source, snapshot, toCanvas, rendition, strip, prepare, release, cancel, bind, audio, encoded, changed, error };
}

afterEach(() => vi.restoreAllMocks());

describe('engine source consumers', () => {
  it.each(['png', 'svg'])('does not request paused transport timing for a %s still', async format => {
    const f = fixture(); Object.assign(f.project.assets.clip, { kind: 'image', format });
    const monitor = f.source.monitor('clip');
    try {
      await Promise.resolve(); await Promise.resolve();
      expect(monitor.duration).toBe(0);
      expect(f.audio).not.toHaveBeenCalled();
      expect(f.error).not.toHaveBeenCalled();
      monitor.pause();
      await Promise.resolve();
      expect(f.audio).not.toHaveBeenCalled();
    } finally { f.source.dispose(); }
  });

  it.each([5, 30])('learns original duration %s while paused and uses it for seeking', async duration => {
    const f = fixture(); f.project.assets.clip.kind = 'audio';
    const monitor = f.source.monitor('clip');
    const changed = vi.fn(), loaded = vi.fn(), ended = vi.fn();
    monitor.addEventListener('durationchange', changed);
    monitor.addEventListener('loadedmetadata', loaded);
    monitor.addEventListener('ended', ended);
    try {
      expect(monitor.duration).toBe(0);
      await vi.waitFor(() => expect(f.audio).toHaveBeenCalledOnce());
      const paused = f.audio.mock.calls[0]![0];
      expect(paused).toMatchObject({ playing: false, time: 0 });
      const status = { audio: 'sourceStatus' as const, asset: 'clip', generation: paused.generation,
        time: 0, playing: false, duration: ticks(duration), levels: { peak: [], rms: [] } };
      f.source.receive(status);
      f.source.receive(status);
      expect(monitor.duration).toBe(duration);
      expect(monitor.paused).toBe(true);
      expect(changed).toHaveBeenCalledOnce(); expect(loaded).toHaveBeenCalledOnce();
      monitor.currentTime = duration + 10;
      await vi.waitFor(() => expect(f.audio.mock.calls.at(-1)![0]).toMatchObject({ time: ticks(duration), playing: false }));
      const seeking = f.audio.mock.calls.at(-1)![0];
      f.source.receive({ ...status, generation: seeking.generation, time: ticks(duration) });
      expect(monitor.currentTime).toBe(duration);
      expect(ended).not.toHaveBeenCalled();
      expect(f.project.assets.clip.dur).toBe(20);
    } finally { f.source.dispose(); }
  });

  it('keeps a seek beyond imported duration while original timing is pending', async () => {
    const f = fixture(); f.project.assets.clip.kind = 'audio';
    const monitor = f.source.monitor('clip');
    try {
      monitor.currentTime = 25;
      await vi.waitFor(() => expect(f.audio.mock.calls.at(-1)![0]).toMatchObject({ time: ticks(25), playing: false }));
      const command = f.audio.mock.calls.at(-1)![0];
      f.source.receive({ audio: 'sourceStatus', asset: 'clip', generation: command.generation,
        time: ticks(25), playing: false, duration: ticks(30), levels: { peak: [], rms: [] } });
      expect(monitor.currentTime).toBe(25);
      expect(monitor.duration).toBe(30);
      expect(f.project.assets.clip.dur).toBe(20);
    } finally { f.source.dispose(); }
  });

  it.each([5, 30])('emits ended at original endpoint %s rather than imported duration', async duration => {
    const f = fixture(); f.project.assets.clip.kind = 'audio';
    const monitor = f.source.monitor('clip');
    const ended = vi.fn(), paused = vi.fn();
    monitor.addEventListener('ended', ended); monitor.addEventListener('pause', paused);
    try {
      await monitor.play();
      const playing = f.audio.mock.calls.at(-1)![0];
      f.source.receive({ audio: 'sourceStatus', asset: 'clip', generation: playing.generation,
        time: 0, playing: true, duration: ticks(duration), levels: { peak: [], rms: [] } });
      monitor.currentTime = duration - 1;
      await vi.waitFor(() => expect(f.audio.mock.calls.at(-1)![0].generation).toBeGreaterThan(playing.generation));
      const seeking = f.audio.mock.calls.at(-1)![0];
      f.source.receive({ audio: 'sourceStatus', asset: 'clip', generation: seeking.generation,
        time: ticks(duration), playing: false, duration: ticks(duration), levels: { peak: [], rms: [] } });
      expect(ended).toHaveBeenCalledOnce(); expect(paused).not.toHaveBeenCalled();
      expect(monitor.currentTime).toBe(duration); expect(monitor.paused).toBe(true);
      expect(f.project.assets.clip.dur).toBe(20);
    } finally { f.source.dispose(); }
  });

  it('does not infer ended from imported metadata or an error status', async () => {
    const f = fixture(); f.project.assets.clip.kind = 'audio';
    const monitor = f.source.monitor('clip');
    const ended = vi.fn(), paused = vi.fn();
    monitor.addEventListener('ended', ended); monitor.addEventListener('pause', paused);
    try {
      await monitor.play();
      f.source.receive({ audio: 'sourceStatus', asset: 'clip', generation: f.audio.mock.calls.at(-1)![0].generation,
        time: ticks(20), playing: false, levels: { peak: [], rms: [] } });
      expect(ended).not.toHaveBeenCalled(); expect(paused).toHaveBeenCalledOnce();
      await monitor.play();
      f.source.receive({ audio: 'sourceStatus', asset: 'clip', generation: f.audio.mock.calls.at(-1)![0].generation,
        time: ticks(5), playing: false, duration: ticks(5), error: 'Source unavailable', levels: { peak: [], rms: [] } });
      expect(ended).not.toHaveBeenCalled(); expect(paused).toHaveBeenCalledTimes(2);
      expect(f.error).toHaveBeenCalledOnce();
    } finally { f.source.dispose(); }
  });

  it('runs source audio on the engine and rejects older status generations after a seek', async () => {
    const f = fixture(); f.project.assets.clip.kind = 'audio';
    const monitor = f.source.monitor('clip');
    await monitor.play();
    const playing = f.audio.mock.calls.at(-1)![0];
    expect(playing).toMatchObject({ audio: 'sourceMonitor', asset: 'clip', playing: true });
    monitor.currentTime = 5;
    await vi.waitFor(() => expect(f.audio.mock.calls.at(-1)![0].generation).toBeGreaterThan(playing.generation));
    const seeking = f.audio.mock.calls.at(-1)![0];
    f.source.receive({ audio: 'sourceStatus', asset: 'clip', time: 0, playing: false, generation: playing.generation, levels: { peak: [0], rms: [0] } });
    expect(monitor.paused).toBe(false);
    f.source.receive({ audio: 'sourceStatus', asset: 'clip', time: 705_600_000 * 5, playing: false, generation: seeking.generation, levels: { peak: [.5], rms: [.2] } });
    expect(monitor.currentTime).toBe(5);
    expect(monitor.levels.peak).toEqual([.5]);
    monitor.dispose();
    expect(f.audio.mock.calls.at(-1)![0]).toMatchObject({ audio: 'sourceMonitor', asset: null, playing: false });
    expect(f.snapshot).not.toHaveBeenCalled();
    f.source.dispose();
  });
  it('uses the shipped poster time, dimensions and encoder without touching a media element', async () => {
    const f = fixture();
    expect(sourcePosterSize(f.project.assets.clip)).toEqual({ width: 320, height: 180 });
    expect((await f.source.poster('clip'))?.type).toBe('image/webp');
    expect(f.snapshot).toHaveBeenCalledWith(expect.objectContaining({ time: .5, width: 320, height: 180, pixels: 'straight', source: { asset: 'clip', fit: undefined, color: undefined } }), 'async');
    expect(f.encoded).toEqual(['image/webp']);
    f.source.dispose();
  });

  it('shares a default poster capture between persistence and inline URL consumers', async () => {
    const create = vi.spyOn(URL, 'createObjectURL').mockReturnValue('blob:poster');
    const revoke = vi.spyOn(URL, 'revokeObjectURL').mockImplementation(() => {});
    const f = fixture();
    let finish!: (value: EngineSnapshot) => void;
    f.snapshot.mockImplementation(() => new Promise(resolve => { finish = resolve; }));
    const persisted = f.source.poster('clip');
    expect(f.source.posterUrl('clip')).toBe('');
    await vi.waitFor(() => expect(f.snapshot).toHaveBeenCalledOnce());
    finish({ width: 320, height: 180, data: new Uint8ClampedArray(320 * 180 * 4) } as EngineSnapshot);
    expect((await persisted)?.type).toBe('image/webp');
    await vi.waitFor(() => expect(f.changed).toHaveBeenCalledOnce());
    (f.project.assets.clip as any).poster = true;
    await f.source.poster('clip');
    expect(f.source.posterUrl('clip')).toBe('blob:poster');
    expect(f.snapshot).toHaveBeenCalledOnce();
    expect(f.toCanvas).toHaveBeenCalledOnce();
    expect(f.encoded).toEqual(['image/webp']);
    expect(create).toHaveBeenCalledOnce();
    f.source.dispose();
    expect(revoke).toHaveBeenCalledWith('blob:poster');
  });

  it('keeps a shared poster capture alive when one caller aborts', async () => {
    vi.spyOn(URL, 'createObjectURL').mockReturnValue('blob:poster');
    vi.spyOn(URL, 'revokeObjectURL').mockImplementation(() => {});
    const f = fixture();
    let finish!: (value: EngineSnapshot) => void;
    f.snapshot.mockImplementation(() => new Promise(resolve => { finish = resolve; }));
    const controller = new AbortController();
    const persistence = f.source.poster('clip', 320, controller.signal);
    const rejected = expect(persistence).rejects.toMatchObject({ name: 'AbortError' });
    f.source.posterUrl('clip');
    await vi.waitFor(() => expect(f.snapshot).toHaveBeenCalledOnce());
    controller.abort();
    await rejected;
    finish({ width: 320, height: 180, data: new Uint8ClampedArray(320 * 180 * 4) } as EngineSnapshot);
    await vi.waitFor(() => expect(f.changed).toHaveBeenCalledOnce());
    expect(f.toCanvas).toHaveBeenCalledOnce();
    expect(f.error).not.toHaveBeenCalled();
    f.source.dispose();
  });

  it('aborts a replaced poster while its binding is unresolved and resets the new capture', async () => {
    const f = fixture();
    f.bind.mockImplementation(() => new Promise<void>(() => {}));
    const old = f.source.poster('clip');
    const oldAborted = expect(old).rejects.toMatchObject({ name: 'AbortError' });
    await vi.waitFor(() => expect(f.bind).toHaveBeenCalledOnce());
    f.project.assets.clip.contentHash = 'replacement';
    const next = f.source.poster('clip');
    const nextAborted = expect(next).rejects.toMatchObject({ name: 'AbortError' });
    // Neither binding resolves: the old promise must reject solely because
    // replacing its identity aborts the controller before map replacement.
    await oldAborted;
    await vi.waitFor(() => expect(f.bind).toHaveBeenCalledTimes(2));
    f.source.reset();
    await nextAborted;
    expect(f.snapshot).not.toHaveBeenCalled();
    expect(f.toCanvas).not.toHaveBeenCalled();
    f.source.dispose();
  });

  it('rejects stale poster captures after reset and preserves custom-edge rendering', async () => {
    const create = vi.spyOn(URL, 'createObjectURL').mockReturnValue('blob:poster');
    vi.spyOn(URL, 'revokeObjectURL').mockImplementation(() => {});
    const f = fixture();
    const encoders: Array<(blob: Blob) => void> = [];
    f.toCanvas.mockReturnValue({ toBlob: (answer: (blob: Blob) => void) => { encoders.push(answer); } });
    const persisted = f.source.poster('clip');
    const rejection = expect(persisted).rejects.toMatchObject({ name: 'AbortError' });
    expect(f.source.posterUrl('clip')).toBe('');
    await vi.waitFor(() => expect(encoders).toHaveLength(1));
    f.source.reset();
    expect(f.source.posterUrl('clip')).toBe('');
    await vi.waitFor(() => expect(encoders).toHaveLength(2));
    encoders[0]!(new Blob(['stale'], { type: 'image/webp' }));
    await rejection;
    await Promise.resolve(); await Promise.resolve();
    expect(create).not.toHaveBeenCalled();
    expect(f.changed).not.toHaveBeenCalled();
    expect(f.error).not.toHaveBeenCalled();
    expect(f.source.posterUrl('clip')).toBe('');
    expect(f.toCanvas).toHaveBeenCalledTimes(2);
    encoders[1]!(new Blob(['fresh'], { type: 'image/webp' }));
    await vi.waitFor(() => expect(f.changed).toHaveBeenCalledOnce());
    f.toCanvas.mockImplementation(() => ({ toBlob: (answer: (blob: Blob) => void, type: string) => answer(new Blob(['fresh'], { type })) }));
    await f.source.poster('clip', 160);
    await f.source.poster('clip', 160);
    expect(f.snapshot).toHaveBeenCalledWith(expect.objectContaining({ width: 160, height: 90 }), 'async');
    expect(f.toCanvas).toHaveBeenCalledTimes(4);
    f.source.dispose();
  });

  it('caches source identity and presentation options, and never keeps replaced media', async () => {
    const f = fixture();
    await f.source.frame('clip', 1, 4, 2);
    await f.source.frame('clip', 1, 4, 2);
    expect(f.snapshot).toHaveBeenCalledTimes(1);
    await f.source.frame('clip', 1, 4, 2, { fit: 'cover' });
    f.project.assets.clip.contentHash = 'replacement';
    await f.source.frame('clip', 1, 4, 2);
    expect(f.snapshot).toHaveBeenCalledTimes(3);
    f.project.assets.clip.rendition = true;
    await f.source.frame('clip', 1, 4, 2);
    expect(f.snapshot).toHaveBeenCalledTimes(4);
    f.project.assets.clip.w = 640;
    await f.source.frame('clip', 1, 4, 2);
    expect(f.snapshot).toHaveBeenCalledTimes(5);
    expect(f.source.stats.hits).toBe(1);
    f.source.dispose();
  });

  it('rejects a cancelled queued frame and a result from an earlier source identity', async () => {
    const f = fixture();
    let finish!: (value: EngineSnapshot) => void;
    f.snapshot.mockImplementation(() => new Promise(resolve => { finish = resolve; }));
    const controller = new AbortController();
    const cancelled = f.source.frame('clip', 1, 4, 2, { signal: controller.signal });
    await vi.waitFor(() => expect(f.snapshot).toHaveBeenCalled());
    controller.abort();
    await expect(cancelled).rejects.toMatchObject({ name: 'AbortError' });
    finish({ data: new Uint8ClampedArray(32) } as EngineSnapshot);
    const stale = f.source.frame('clip', 2, 4, 2);
    await vi.waitFor(() => expect(f.snapshot).toHaveBeenCalledTimes(2));
    f.project.assets.clip.contentHash = 'replacement';
    finish({ data: new Uint8ClampedArray(32) } as EngineSnapshot);
    await expect(stale).rejects.toMatchObject({ name: 'AbortError' });
    expect(f.source.stats.frames).toBe(0);
    f.source.dispose();
  });

  it('drops native planes delivered after their frame caller has aborted', async () => {
    const f = fixture();
    let finish!: (value: EngineSnapshot) => void;
    f.snapshot.mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
    const controller = new AbortController();
    const cancelled = f.source.frame('clip', 1, 4, 2, { color: 'compositor', signal: controller.signal });
    await vi.waitFor(() => expect(f.snapshot).toHaveBeenCalledOnce());
    controller.abort();
    await expect(cancelled).rejects.toMatchObject({ name: 'AbortError' });
    finish({ width: 4, height: 2, data: new Uint8ClampedArray(12),
      videoPlanes: { format: 'nv12', primaries: 'bt709', transfer: 'bt709', matrix: 'bt709', range: 'limited', timestamp: 1 } } as EngineSnapshot);
    await Promise.resolve();
    expect(f.source.stats.frames).toBe(0);
    expect(f.source.stats.hits).toBe(0);
    await f.source.frame('clip', 1, 4, 2, { color: 'compositor' });
    expect(f.snapshot).toHaveBeenCalledTimes(2);
    expect(f.error).not.toHaveBeenCalled();
    f.source.dispose();
  });

  it('never aliases compositor video colour with Canvas2D poster pixels', async () => {
    const f = fixture();
    try {
      await f.source.frame('clip', 1, 4, 2, { color: 'display' });
      await f.source.frame('clip', 1, 4, 2, { color: 'compositor' });
      await f.source.frame('clip', 1, 4, 2, { color: 'compositor' });
      expect(f.snapshot).toHaveBeenCalledTimes(3);
      await f.source.frame('clip', 1, 4, 2, { color: 'display' });
      expect(f.snapshot).toHaveBeenCalledTimes(3);
      expect(f.snapshot).toHaveBeenNthCalledWith(1, expect.objectContaining({ source: expect.objectContaining({ color: 'display' }) }), 'async');
      expect(f.snapshot).toHaveBeenNthCalledWith(2, expect.objectContaining({ source: expect.objectContaining({ color: 'compositor' }) }), 'async');
      expect(f.source.stats.hits).toBe(1);
    } finally { f.source.dispose(); }
  });

  it('freezes the original before waiting for bindings and cancels the owning job', async () => {
    const f = fixture();
    let bound!: () => void;
    f.bind.mockImplementation(() => new Promise(resolve => { bound = resolve; }));
    const controller = new AbortController();
    let finish!: (value: { width: number; height: number; count: number; data: Uint8Array }) => void;
    f.strip.mockImplementation(() => new Promise(resolve => { finish = resolve; }));
    const result = f.source.filmstrip('clip', [2, 0], 4, 2, { signal: controller.signal });
    f.project.assets.clip.contentHash = 'replacement'; bound();
    await vi.waitFor(() => expect(f.strip).toHaveBeenCalled());
    const request = f.strip.mock.calls[0]![0] as any;
    expect(JSON.parse(request.project).assets.clip.contentHash).toBe('original');
    expect(request.times).toEqual([2, 0]);
    controller.abort();
    await expect(result).rejects.toMatchObject({ name: 'AbortError' });
    expect(f.cancel).toHaveBeenCalledWith(request.requestId);
    finish({ width: 4, height: 2, count: 0, data: new Uint8Array() });
    f.source.dispose();
  });

  it('reuses acknowledged source bindings until replacement or reset', async () => {
    const f = fixture();
    await f.source.frame('clip', 1, 4, 2);
    await f.source.frame('clip', 2, 4, 2);
    expect(f.bind).toHaveBeenCalledTimes(1);
    f.project.assets.clip.contentHash = 'new-original';
    await f.source.frame('clip', 3, 4, 2);
    expect(f.bind).toHaveBeenCalledTimes(2);
    f.source.reset();
    await f.source.frame('clip', 3, 4, 2);
    expect(f.bind).toHaveBeenCalledTimes(3);
    f.source.dispose();
  });

  it('caches packed filmstrips without allowing callers to mutate later frames', async () => {
    const f = fixture();
    const frames = await f.source.filmstrip('clip', [0, 1], 4, 2);
    expect(frames.map(frame => frame.byteLength)).toEqual([32, 32]);
    frames[0]![0] = 255;
    const again = await f.source.filmstrip('clip', [0, 1], 4, 2);
    expect(again[0]![0]).toBe(0);
    expect(f.strip).toHaveBeenCalledTimes(1);
    f.project.assets.clip.rendition = true;
    await f.source.filmstrip('clip', [0, 1], 4, 2);
    expect(f.strip).toHaveBeenCalledTimes(2);
    f.source.dispose();
  });

  it('rejects incomplete packed filmstrips and never caches them', async () => {
    const f = fixture();
    f.strip.mockResolvedValue({ width: 4, height: 2, count: 2, data: new Uint8Array(32) });
    await expect(f.source.filmstrip('clip', [0, 1], 4, 2)).rejects.toThrow('incomplete filmstrip');
    await expect(f.source.filmstrip('clip', [0, 1], 4, 2)).rejects.toThrow('incomplete filmstrip');
    expect(f.strip).toHaveBeenCalledTimes(2);
    f.source.dispose();
  });

  it('prepares real source frames from poster use and retires them on reset', async () => {
    const f = fixture();
    await f.source.poster('clip');
    await vi.waitFor(() => expect(f.prepare).toHaveBeenCalledOnce());
    expect(f.prepare).toHaveBeenCalledWith(expect.objectContaining({ asset: 'clip', time: .5, width: 320, height: 180, fit: 'stretch', color: 'display' }));
    f.source.reset();
    expect(f.release).toHaveBeenCalledOnce();
    f.source.dispose();
  });

  it('uses observed CSS size without forcing layout on each source seek', async () => {
    let resized!: ResizeObserverCallback;
    vi.stubGlobal('ResizeObserver', class {
      constructor(callback: ResizeObserverCallback) { resized = callback; }
      observe() {} disconnect() {}
    });
    const f = fixture();
    try {
      const monitor = f.source.monitor('clip');
      const width = vi.spyOn(monitor.surface, 'clientWidth', 'get').mockReturnValue(160);
      const height = vi.spyOn(monitor.surface, 'clientHeight', 'get').mockReturnValue(90);
      resized([{ target: monitor.surface } as unknown as ResizeObserverEntry], {} as ResizeObserver);
      await vi.waitFor(() => expect(f.snapshot).toHaveBeenCalled());
      await Promise.resolve(); await Promise.resolve();
      width.mockClear(); height.mockClear();
      monitor.currentTime = 3;
      await vi.waitFor(() => expect(f.snapshot).toHaveBeenCalledWith(expect.objectContaining({ time: 3,
        width: Math.round(160 * devicePixelRatio), height: Math.round(90 * devicePixelRatio) }), 'async'));
      expect(width).not.toHaveBeenCalled(); expect(height).not.toHaveBeenCalled();
    } finally { f.source.dispose(); vi.unstubAllGlobals(); }
  });

  it.each([
    [{ alpha: 'premultiplied' }, 'png', 'interpreted'],
    [undefined, 'png', 'display'],
    [{ alpha: 'premultiplied' }, 'svg', 'display']
  ] as const)('reads a still with interpret %j (%s) through the %s reading', async (interpret, format, color) => {
    vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} });
    const f = fixture();
    Object.assign(f.project.assets.clip, { kind: 'image', format, interpret });
    let surface: HTMLElement | null = null;
    try {
      const monitor = f.source.monitor('clip');
      surface = monitor.surface;
      document.body.append(monitor.surface);
      await vi.waitFor(() => expect(f.snapshot).toHaveBeenCalledWith(
        expect.objectContaining({ source: expect.objectContaining({ color }) }), 'async'));
    } finally { f.source.dispose(); surface?.remove(); vi.unstubAllGlobals(); }
  });

  it('repaints an open paused still after interpretation changes and drops its old paint', async () => {
    vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} });
    vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockImplementation((() => ({ putImageData: vi.fn() })) as unknown as HTMLCanvasElement['getContext']);
    vi.spyOn(HTMLCanvasElement.prototype, 'toBlob').mockImplementation(callback => callback(new Blob(['frame'], { type: 'image/png' })));
    vi.spyOn(HTMLImageElement.prototype, 'decode').mockResolvedValue();
    vi.spyOn(URL, 'createObjectURL').mockReturnValue('blob:frame');
    vi.spyOn(URL, 'revokeObjectURL').mockImplementation(() => {});
    const f = fixture();
    Object.assign(f.project.assets.clip, { kind: 'image', format: 'png', w: 4, h: 2 });
    let finishOld!: (value: EngineSnapshot) => void;
    f.snapshot.mockImplementationOnce(() => new Promise(resolve => { finishOld = resolve; }));
    let surface: HTMLElement | null = null;
    try {
      const monitor = f.source.monitor('clip');
      surface = monitor.surface;
      const loaded = vi.fn();
      monitor.addEventListener('loadeddata', loaded);
      document.body.append(monitor.surface);
      await vi.waitFor(() => expect(f.snapshot).toHaveBeenCalledOnce());
      (f.project.assets.clip as any).interpret = { alpha: 'premultiplied' };
      f.source.assetsChanged();
      finishOld({ width: 4, height: 2, data: new Uint8ClampedArray(32) } as EngineSnapshot);
      await vi.waitFor(() => expect(f.snapshot).toHaveBeenCalledTimes(2));
      expect(f.snapshot).toHaveBeenNthCalledWith(2, expect.objectContaining({ source: {
        asset: 'clip', fit: 'contain', color: 'interpreted'
      } }), 'async');
      await vi.waitFor(() => expect(loaded).toHaveBeenCalledOnce());
      expect(monitor.paused).toBe(true);
      f.source.assetsChanged();
      await Promise.resolve();
      expect(f.snapshot).toHaveBeenCalledTimes(2);
      expect(f.error).not.toHaveBeenCalled();
    } finally { f.source.dispose(); surface?.remove(); vi.unstubAllGlobals(); }
  });

  it('does not wait for a cancelled image encode before painting the new reading', async () => {
    vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} });
    vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockImplementation((() => ({ putImageData: vi.fn() })) as unknown as HTMLCanvasElement['getContext']);
    const encodes: Array<(blob: Blob | null) => void> = [];
    vi.spyOn(HTMLCanvasElement.prototype, 'toBlob').mockImplementation(callback => { encodes.push(callback); });
    vi.spyOn(HTMLImageElement.prototype, 'decode').mockResolvedValue();
    const url = vi.spyOn(URL, 'createObjectURL').mockReturnValue('blob:frame');
    vi.spyOn(URL, 'revokeObjectURL').mockImplementation(() => {});
    const f = fixture();
    Object.assign(f.project.assets.clip, { kind: 'image', format: 'png', w: 4, h: 2 });
    let surface: HTMLElement | null = null;
    try {
      const monitor = f.source.monitor('clip');
      surface = monitor.surface;
      const loaded = vi.fn();
      monitor.addEventListener('loadeddata', loaded);
      document.body.append(monitor.surface);
      await vi.waitFor(() => expect(encodes).toHaveLength(1));
      (f.project.assets.clip as any).interpret = { alpha: 'premultiplied' };
      f.source.assetsChanged();
      await vi.waitFor(() => expect(encodes).toHaveLength(2));
      encodes[1]!(new Blob(['fresh'], { type: 'image/png' }));
      await vi.waitFor(() => expect(loaded).toHaveBeenCalledOnce());
      encodes[0]!(new Blob(['stale'], { type: 'image/png' }));
      await Promise.resolve();
      expect(url).toHaveBeenCalledOnce();
      expect(f.error).not.toHaveBeenCalled();
    } finally { f.source.dispose(); surface?.remove(); vi.unstubAllGlobals(); }
  });

  it('replaces pending video planes after an interpretation edit', async () => {
    vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} });
    const f = fixture();
    const present = vi.spyOn(SourceVideoPresenter.prototype, 'paint').mockResolvedValue();
    let finishOld!: (value: EngineSnapshot) => void;
    f.snapshot.mockImplementationOnce(() => new Promise(resolve => { finishOld = resolve; }));
    const video = (timestamp: number) => ({ format: 'nv12', primaries: 'bt709', transfer: 'bt709', matrix: 'bt709', range: 'limited', timestamp });
    const fresh = { width: 4, height: 2, data: new Uint8ClampedArray(12), videoPlanes: video(2) } as EngineSnapshot;
    f.snapshot.mockResolvedValue(fresh);
    let surface: HTMLElement | null = null;
    try {
      const monitor = f.source.monitor('clip');
      surface = monitor.surface;
      document.body.append(monitor.surface);
      await vi.waitFor(() => expect(f.snapshot).toHaveBeenCalledOnce());
      (f.project.assets.clip as any).interpret = { primaries: 'bt2020' };
      f.source.assetsChanged();
      finishOld({ width: 4, height: 2, data: new Uint8ClampedArray(12), videoPlanes: video(1) } as EngineSnapshot);
      await vi.waitFor(() => expect(present).toHaveBeenCalledOnce());
      expect(present).toHaveBeenCalledWith({ video: fresh.videoPlanes, data: fresh.data, width: 4, height: 2 }, expect.any(AbortSignal));
      expect(f.snapshot).toHaveBeenNthCalledWith(2, expect.objectContaining({ source: {
        asset: 'clip', fit: 'contain', color: 'compositor'
      } }), 'async');
    } finally { f.source.dispose(); surface?.remove(); vi.unstubAllGlobals(); }
  });

  it('keeps source transport time and playback while retiring status from the old reading', async () => {
    vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} });
    const f = fixture();
    f.project.assets.clip.kind = 'audio';
    try {
      const monitor = f.source.monitor('clip');
      monitor.currentTime = 3;
      await vi.waitFor(() => expect(f.audio.mock.calls.at(-1)![0]).toMatchObject({ time: ticks(3), playing: false }));
      await monitor.play();
      const old = f.audio.mock.calls.at(-1)![0];
      Object.assign(f.project.assets.clip, { interpret: { frameRate: 30 }, dur: 10 });
      f.source.assetsChanged();
      await vi.waitFor(() => expect(f.audio.mock.calls.at(-1)![0].generation).toBeGreaterThan(old.generation));
      const fresh = f.audio.mock.calls.at(-1)![0];
      expect(fresh).toMatchObject({ asset: 'clip', playing: true });
      expect(fresh.time).toBeGreaterThanOrEqual(ticks(3));
      expect(monitor.duration).toBe(0);
      f.source.receive({ audio: 'sourceStatus', asset: 'clip', time: 0, playing: false,
        generation: old.generation, duration: ticks(1), levels: { peak: [], rms: [] } });
      expect(monitor.paused).toBe(false);
      expect(monitor.duration).toBe(0);
      expect(monitor.currentTime).toBeGreaterThanOrEqual(3);
      f.source.receive({ audio: 'sourceStatus', asset: 'clip', time: ticks(3), playing: true,
        generation: fresh.generation, duration: ticks(30), levels: { peak: [], rms: [] } });
      expect(monitor.duration).toBe(30);
      expect(f.project.assets.clip.dur).toBe(10);
    } finally { f.source.dispose(); vi.unstubAllGlobals(); }
  });

  it('preserves image-element compositing for native stills', () => {
    const f = fixture(); f.project.assets.clip.kind = 'image';
    const monitor = f.source.monitor('clip');
    expect(monitor.surface).toBeInstanceOf(HTMLImageElement);
    f.source.dispose();
  });
});
