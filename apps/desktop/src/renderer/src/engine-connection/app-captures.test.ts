// @vitest-environment happy-dom
import { describe, expect, it, vi } from 'vitest';

import { install as installExporter } from '../legacy/core/exporter';
import { installAppCaptures } from './app-captures';

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
  snapshotDataUrl: vi.fn(async (time: number, width: number) => `data:engine-async:${time}:${width}`)
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
    expect(e.snapshotDataUrlSync).toHaveBeenCalledWith(1.5, 320, PM.proj);
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
    expect(e.snapshotDataUrlSync).toHaveBeenCalledWith(0, 480, PM.proj);
  });
});
