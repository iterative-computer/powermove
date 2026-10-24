// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { EngineSourceMedia, sourcePosterSize } from './source-media';
import type { EngineSnapshot, EngineRenderTransport } from './render-transport';
import type { EngineBridge } from '../../../shared/engine-ipc';
import type { FrameBridgeApi } from '../../../shared/frame-bridge';
import type { DocumentSession } from './session';

function fixture(frameBridge?: FrameBridgeApi) {
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
    frameBridge,
    render: { snapshot, toCanvas } as unknown as EngineRenderTransport,
    native: { browserRendition: rendition, sourceFilmstrip: strip, prepareSource: prepare, releasePreparedSources: release, cancelMediaJob: cancel } as unknown as EngineBridge,
    session: () => ({ audio } as unknown as DocumentSession), wake: async () => {}, bind,
    changed, error
  });
  return { project, source, snapshot, toCanvas, rendition, strip, prepare, release, cancel, bind, audio, encoded, changed, error };
}

afterEach(() => vi.restoreAllMocks());

describe('engine source consumers', () => {
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

  it('releases a native surface delivered after its frame caller has aborted', async () => {
    const discardSource = vi.fn();
    const f = fixture({ discardSource } as unknown as FrameBridgeApi);
    let finish!: (value: EngineSnapshot) => void;
    f.snapshot.mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
    const controller = new AbortController();
    const cancelled = f.source.frame('clip', 1, 4, 2, { color: 'compositor', signal: controller.signal });
    await vi.waitFor(() => expect(f.snapshot).toHaveBeenCalledOnce());
    controller.abort();
    await expect(cancelled).rejects.toMatchObject({ name: 'AbortError' });
    finish({ width: 4, height: 2, data: new Uint8ClampedArray(),
      videoSurface: { token: 'late-surface', width: 4, height: 2 } } as EngineSnapshot);
    await vi.waitFor(() => expect(discardSource).toHaveBeenCalledOnce());
    expect(discardSource).toHaveBeenCalledWith('late-surface');
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
      vi.spyOn(monitor.canvas, 'getContext').mockReturnValue(null);
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

  it('preserves image-element compositing for native stills', () => {
    const f = fixture(); f.project.assets.clip.kind = 'image';
    const monitor = f.source.monitor('clip');
    expect(monitor.surface).toBeInstanceOf(HTMLImageElement);
    expect(monitor.surface).not.toBe(monitor.canvas);
    f.source.dispose();
  });
});
