// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { EngineSourceMedia, sourcePosterSize } from './source-media';
import type { EngineSnapshot, EngineRenderTransport } from './render-transport';
import type { EngineBridge } from '../../../shared/engine-ipc';
import type { DocumentSession } from './session';

function fixture() {
  const project = { id: 'project', assets: { clip: { id: 'clip', kind: 'video', w: 1920, h: 1080, dur: 20, contentHash: 'original' } } };
  const pixels = { width: 4, height: 2, data: new Uint8ClampedArray(32), revision: 1, pixels: 'straight' } as EngineSnapshot;
  const snapshot = vi.fn(async () => pixels);
  const encoded: string[] = [];
  const toCanvas = vi.fn(() => ({ toBlob: (answer: (blob: Blob) => void, type: string) => { encoded.push(type); answer(new Blob(['poster'], { type })); } }));
  const rendition = vi.fn(async () => ({ data: new Uint8Array([1, 2]), type: 'video/webm' as const }));
  const strip = vi.fn(async (_request: unknown): Promise<{ width: number; height: number; frames: Uint8Array[] }> => ({ width: 4, height: 2, frames: [new Uint8Array(32), new Uint8Array(32)] }));
  const cancel = vi.fn(async () => {}), audio = vi.fn();
  const bind = vi.fn(async () => {});
  const source = new EngineSourceMedia({ project: () => project, revision: () => 7,
    render: { snapshot, toCanvas } as unknown as EngineRenderTransport,
    native: { browserRendition: rendition, sourceFilmstrip: strip, cancelMediaJob: cancel } as unknown as EngineBridge,
    session: () => ({ audio } as unknown as DocumentSession), wake: async () => {}, bind,
    changed: vi.fn(), error: vi.fn()
  });
  return { project, source, snapshot, rendition, strip, cancel, bind, audio, encoded };
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

  it('caches source identity and presentation options, and never keeps replaced media', async () => {
    const f = fixture();
    await f.source.frame('clip', 1, 4, 2);
    await f.source.frame('clip', 1, 4, 2);
    expect(f.snapshot).toHaveBeenCalledTimes(1);
    await f.source.frame('clip', 1, 4, 2, { fit: 'cover' });
    f.project.assets.clip.contentHash = 'replacement';
    await f.source.frame('clip', 1, 4, 2);
    expect(f.snapshot).toHaveBeenCalledTimes(3);
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

  it('freezes the original before waiting for bindings and cancels the owning job', async () => {
    const f = fixture();
    let bound!: () => void;
    f.bind.mockImplementation(() => new Promise(resolve => { bound = resolve; }));
    const controller = new AbortController();
    let finish!: (value: { width: number; height: number; frames: Uint8Array[] }) => void;
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
    finish({ width: 4, height: 2, frames: [] });
    f.source.dispose();
  });
});
