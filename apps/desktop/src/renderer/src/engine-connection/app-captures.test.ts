// @vitest-environment happy-dom
import { describe, expect, it, vi } from 'vitest';

import { install as installExporter } from '../legacy/core/exporter';
import { CAPTURE_BUSY, installAppCaptures } from './app-captures';

vi.mock('../legacy/core/export-preview', () => ({ animateExportPreview: () => () => {} }));
vi.mock('../player/export-web', () => ({
  buildWebExport: async () => ({ bytes: new Uint8Array([1]), scene: { warnings: [] } }),
  inspectWebExport: () => ({ errors: [], fonts: new Set() })
}));

function legacyPM() {
  const renderFrameTo = vi.fn(() => ({ toDataURL: () => 'data:legacy' }));
  const PM: any = {
    version: 'test', h: () => document.createElement('div'), proj: { name: 'Test', w: 640, h: 360, fps: 30, dur: 1, layers: [] },
    store: { get: () => ({}), set() {} }, toast() {}, download: async () => ({ ok: true }),
    modal: () => ({ el: document.createElement('div'), close() {} }),
    scope: [], pause() {}, play() {}, setTime() {}, time: 0, playing: false, quality: 1,
    round: (n: number) => n, tc: () => '0:00', invalidate() {},
    Audio: { hasAudibleLayers: () => false },
    GL: { canvas: {}, resize() {}, render() {}, renderToPixels: () => new Uint8Array(16) },
    renderFrameTo
  };
  return { PM, renderFrameTo };
}

const engine = () => ({
  snapshotDataUrlSync: vi.fn((time: number, width: number) => `data:engine-sync:${time}:${width}`),
  snapshotDataUrl: vi.fn(async (time: number, width: number, _size?: unknown, _options?: { captionsHidden?: boolean }) => `data:engine-async:${time}:${width}`)
});

describe('the app’s captures with the new engine on', () => {
  it('reach the engine although the exporter installs after the connection (bootstrap order)', async () => {
    const { PM, renderFrameTo } = legacyPM();
    const e = engine();
    // `engine/connection` installs before `core/exporter` (legacy/bootstrap.ts).
    const dispose = installAppCaptures(PM, e);
    installExporter(PM);
    expect(PM.Export.snapshot(1.5, 320)).toBe('data:engine-sync:1.5:320');
    expect(await PM.Export.snapshotAsync(2, 200)).toBe('data:engine-async:2:200');
    expect(e.snapshotDataUrlSync).toHaveBeenCalledWith(1.5, 320, PM.proj, { captionsHidden: false });
    expect(renderFrameTo).not.toHaveBeenCalled();
    dispose();
    expect(PM.EngineCaptures).toBeNull();
    expect(PM.Export.snapshot(0, 100)).toBe('data:legacy');
  });

  it('reach the engine when the exporter was there first, and through a wrapper an extension installed', () => {
    const { PM, renderFrameTo } = legacyPM();
    installExporter(PM);
    const original = PM.Export.snapshot;
    PM.Export.snapshot = (t: number, w: number) => `wrapped(${original(t, w)})`;
    const e = engine();
    installAppCaptures(PM, e);
    expect(PM.Export.snapshot(0.5, 480)).toBe('wrapped(data:engine-sync:0.5:480)');
    expect(renderFrameTo).not.toHaveBeenCalled();
  });

  it('keep today’s defaults (480 px wide)', () => {
    const { PM } = legacyPM();
    const e = engine();
    installAppCaptures(PM, e);
    installExporter(PM);
    PM.Export.snapshot(0);
    expect(e.snapshotDataUrlSync).toHaveBeenCalledWith(0, 480, PM.proj, { captionsHidden: false });
  });

  it('snapshotAsync keeps today’s contract: playback pauses and resumes, one capture at a time', async () => {
    const { PM } = legacyPM();
    const calls: string[] = [];
    PM.playing = true;
    PM.pause = () => { calls.push('pause'); PM.playing = false; };
    PM.play = () => { calls.push('play'); PM.playing = true; };
    let finish: (url: string) => void = () => {};
    const e = engine();
    e.snapshotDataUrl.mockImplementation(() => { calls.push('capture'); return new Promise<string>((resolve) => { finish = resolve; }); });
    installAppCaptures(PM, e);
    installExporter(PM);
    const pending = PM.Export.snapshotAsync(1, 320);
    await Promise.resolve();
    expect(e.snapshotDataUrl).toHaveBeenCalledWith(1, 320, PM.proj, { captionsHidden: false });
    expect(PM.agentFrameCapture).toBe(true);
    // A second capture while one runs is refused with today's words.
    await expect(PM.Export.snapshotAsync(2, 320)).rejects.toThrow(CAPTURE_BUSY);
    finish('data:engine');
    expect(await pending).toBe('data:engine');
    expect(calls).toEqual(['pause', 'capture', 'play']);
    expect(PM.agentFrameCapture).toBe(false);
    // While an export runs, too.
    PM.Export.busy = true;
    await expect(PM.Export.snapshotAsync(0, 320)).rejects.toThrow(CAPTURE_BUSY);
    PM.Export.busy = false;
    // A failure still resumes playback and frees the slot.
    e.snapshotDataUrl.mockImplementation(async () => { throw new Error('The engine could not draw this frame.'); });
    PM.playing = true;
    calls.length = 0;
    await expect(PM.Export.snapshotAsync(0, 320)).rejects.toThrow(/could not draw/);
    expect(calls).toEqual(['pause', 'play']);
    expect(PM.agentFrameCapture).toBe(false);
  });

  it('snapshotAsync honours the capture signal: refused when aborted, rejected on abort, busy until the draw ends', async () => {
    const { PM } = legacyPM();
    let finish: (url: string) => void = () => {};
    const e = engine();
    e.snapshotDataUrl.mockImplementation(() => new Promise<string>((resolve) => { finish = resolve; }));
    installAppCaptures(PM, e);
    installExporter(PM);
    const early = new AbortController();
    early.abort(new DOMException('gone', 'AbortError'));
    await expect(PM.Export.snapshotAsync(0, 320, { signal: early.signal })).rejects.toThrow('gone');
    expect(e.snapshotDataUrl).not.toHaveBeenCalled();
    const controller = new AbortController();
    const pending = PM.Export.snapshotAsync(1, 320, { signal: controller.signal });
    await Promise.resolve();
    controller.abort();
    await expect(pending).rejects.toMatchObject({ name: 'AbortError' });
    expect(PM.agentFrameCapture).toBe(true);
    finish('data:late');
    await vi.waitFor(() => expect(PM.agentFrameCapture).toBe(false));
  });

  it('reads caption visibility separately for synchronous and asynchronous captures', async () => {
    const { PM } = legacyPM();
    const e = engine();
    installAppCaptures(PM, e);
    for (const hidden of [true, false]) {
      PM.captionsHidden = hidden;
      PM.EngineCaptures.snapshot(0, 320);
      await PM.EngineCaptures.snapshotAsync(0, 320);
      expect(e.snapshotDataUrlSync).toHaveBeenLastCalledWith(0, 320, PM.proj, { captionsHidden: hidden });
      expect(e.snapshotDataUrl).toHaveBeenLastCalledWith(0, 320, PM.proj, { captionsHidden: hidden });
    }
  });
});
