// @vitest-environment happy-dom
import { describe, expect, it, vi } from 'vitest';
import { createExportStats } from './export-stats';
import { ExportVideoDecoders } from './export-video-decoder';

describe('export stage timings', () => {
  it('sums each frame across calls and reports totals and percentiles', async () => {
    const stats = createExportStats(4);
    for (let frame = 0; frame < 3; frame++) { stats.add('decode', frame, 10 * (frame + 1)); stats.add('render', frame, 5); }
    stats.add('render', 2, 5);
    await expect(stats.time('encode', 0, () => { throw new Error('encoder stopped'); })).rejects.toThrow('encoder stopped');
    stats.add('encode', 9, 1); stats.add('decode', 1, NaN);
    stats.repeated = 5; stats.encoder = 'libx264'; stats.audioMs = 12.34;
    const summary = stats.summary({ decoded: 3, seeked: 0, fallbacks: [] });
    expect(summary).toMatchObject({ frames: 8, repeatedFrames: 5, encoder: 'libx264', audioMs: 12.3, video: { decoded: 3, seeked: 0 } });
    expect(summary.stages.decode).toEqual({ totalMs: 60, p50Ms: 20, p95Ms: 30 });
    expect(summary.stages.render).toEqual({ totalMs: 20, p50Ms: 5, p95Ms: 10 });
    expect(summary.stages.encode.totalMs).toBeGreaterThanOrEqual(0);
  });
});

describe('sequential export decoders', () => {
  it('keeps seeking where WebCodecs or the source cannot reproduce <video>', async () => {
    const decoders = new ExportVideoDecoders();
    const el = document.createElement('video');
    Object.defineProperties(el, { videoWidth: { value: 64 }, videoHeight: { value: 36 }, currentSrc: { value: 'blob:clip' } });
    const asset = { id: 'clip', persistBlob: new Blob(['not an mp4']), url: 'blob:clip', el };
    // No VideoDecoder in this environment.
    expect(await decoders.frame(asset, 'layer', 0)).toBeNull();
    // A source the element no longer plays never uses the stored blob.
    expect(await decoders.frame({ ...asset, url: 'blob:replaced' }, 'other', 0)).toBeNull();
    expect(await decoders.frame({ ...asset, imageSequence: {} }, 'sequence', 0)).toBeNull();
    vi.stubGlobal('VideoDecoder', class { static isConfigSupported = vi.fn(); });
    vi.stubGlobal('EncodedVideoChunk', class {});
    try {
      // WebCodecs exists, but this blob is not an MP4 it can walk.
      expect(await decoders.frame(asset, 'unparsed', 0)).toBeNull();
    } finally { vi.unstubAllGlobals(); }
    expect(decoders.decoding(['layer'])).toBe(false);
    expect(decoders.stats).toMatchObject({ decoded: 0, seeked: 4 });
    decoders.dispose();
    expect(await decoders.frame(asset, 'layer', 0)).toBeNull();
  });
});
