import { afterEach, describe, expect, it, vi } from 'vitest';

import {
  FakeAudioBuffer,
  audioAsset,
  audioHarness,
  audioLayer,
  deferred,
  flush,
} from './audio-fakes';

afterEach(() => vi.unstubAllGlobals());

describe('legacy audio system behavior', () => {
  it('decodes imported channel samples and derives reusable waveform peaks', async () => {
    const buffer = new FakeAudioBuffer([
      [0, 0.25, -0.8, 0.1, 0.2, -0.4, 0.1, 0],
      [-0.1, 0.5, 0.3, -0.2, 0.9, 0.1, 0, -0.1],
    ], 4);
    const h = audioHarness({ decodedBuffer: buffer });
    const blob = new Blob([new Uint8Array([82, 73, 70, 70, 1, 2, 3, 4])], { type: 'audio/wav' });

    const asset = await h.PM.Audio.prepareAsset({ id: 'asset-1', name: 'landmarks.wav', blob });

    expect(asset.audioBuffer).toBe(buffer);
    expect(asset.audioBlob).toBe(blob);
    expect([asset.dur, asset.channels, asset.sampleRate]).toEqual([2, 2, 4]);
    expect(Array.from(asset.peaks, (value: any) => Number(value.toFixed(6))))
      .toEqual([0.1, 0.5, 0.8, 0.2, 0.9, 0.4, 0.1, 0.1]);
    expect(h.decodeCalls).toHaveLength(1);
    expect(h.events).toContainEqual(['audio:decoded', 'asset-1']);
    expect(h.invalidations).toContain('timeline');
    await expect(h.PM.Audio.decodeAsset(asset)).resolves.toBe(buffer);
    expect(h.decodeCalls).toHaveLength(1);

    h.assets.set(asset.id, asset);
    const bars: any[][] = [];
    expect(h.PM.Audio.drawWaveform({ fillRect: (...args: any[]) => bars.push(args) }, audioLayer({ dur: 2 }), {
      x: 0, y: 0, width: 20, height: 12, clipLeft: 0, step: 2,
    })).toBe(true);
    expect(bars.some(([, y, , height]) => height > 1 && y < 6)).toBe(true);
  });

  it('rejects oversized decoded audio before it enters the project cache', async () => {
    const huge = {
      numberOfChannels: 2, length: 42_000_000, sampleRate: 48_000, duration: 875,
      getChannelData() { throw new Error('peak generation must not run'); },
    };
    const h = audioHarness({ decodedBuffer: huge });
    const blob = new Blob([new Uint8Array([1, 2, 3, 4])], { type: 'audio/wav' });

    await expect(h.PM.Audio.prepareAsset({ id: 'asset-huge', name: 'too-long.wav', blob }))
      .rejects.toThrow(/too long to decode safely/);
  });

  it('stops waveform painting at source EOF', () => {
    const buffer = new FakeAudioBuffer([[0.2, 0.4, 0.8, 0.1]], 2);
    const h = audioHarness();
    h.assets.set('asset-1', { ...audioAsset(buffer), peaks: new Float32Array([0.2, 0.4, 0.8, 0.1]) });
    const bars: any[][] = [];

    h.PM.Audio.drawWaveform({ fillRect: (...args: any[]) => bars.push(args) }, audioLayer({ dur: 4 }), {
      x: 0, y: 0, width: 40, height: 12, clipLeft: 0, step: 10,
    });

    expect(bars).toHaveLength(2);
  });

  it('plans trim, ranges, source bounds, and enablement consistently', () => {
    const buffer = new FakeAudioBuffer([new Float32Array(20)], 4);
    const { PM } = audioHarness();
    const asset = { ...audioAsset(buffer), dur: 99 };
    const layer = audioLayer({ from: 2, dur: 6, d: { trim: 1 } });

    const clip = PM.Audio.plan(layer, asset, 3.5, 10);
    expect(clip).toMatchObject({ start: 3.5, localStart: 1.5, sourceOffset: 2.5, duration: 2.5, end: 6 });
    expect(PM.Audio.plan(layer, asset, 0, 2)).toBeNull();
    expect(PM.Audio.plan({ ...layer, on: false }, asset, 0, 10)).toBeNull();
    expect(PM.Audio.plan(audioLayer({ from: 2, d: { trim: 5 } }), asset, 3, 6)).toBeNull();
  });

  it('starts once, restarts at exact seeks, pauses, and starts again', () => {
    const buffer = new FakeAudioBuffer([new Float32Array(40)], 4);
    const h = audioHarness();
    const asset = audioAsset(buffer);
    const layer = audioLayer({ from: 2, dur: 5, d: { trim: 1 } });
    h.assets.set(asset.id, asset);
    h.PM.proj.layers = [layer];
    h.PM.time = 3;

    h.PM.Audio.start(3);
    const context = h.contexts[0];
    expect(context.sources[0].starts[0]).toEqual({ when: 0.25, offset: 2, duration: 4 });
    expect(h.PM.Audio.inspect().voices).toEqual([
      { layerId: 'audio-1', assetId: 'asset-1', sourceOffset: 2, duration: 4 },
    ]);
    context.currentTime += 0.5;
    h.PM.Audio.tick(3.5);
    expect(context.sources).toHaveLength(1);

    h.PM.Audio.seek(5);
    expect(context.sources[0].stopCalls).toBe(1);
    expect(context.sources[1].starts[0]).toEqual({ when: 0.75, offset: 4, duration: 2 });
    h.PM.Audio.pause();
    expect(context.sources[1].stopCalls).toBe(1);
    expect(h.PM.Audio.inspect()).toMatchObject({ running: false, voices: [] });

    h.PM.Audio.start(4);
    expect(context.sources[2].starts[0]).toEqual({ when: 0.75, offset: 3, duration: 3 });
    h.PM.Audio.tick(7);
    expect(context.sources[2].stopCalls).toBe(1);
    expect(h.PM.Audio.inspect().voices).toEqual([]);
  });

  it('mixes an attached video soundtrack before it is separated', () => {
    const buffer = new FakeAudioBuffer([new Float32Array(40)], 4);
    const h = audioHarness();
    h.PM.evP = (_layer: any, prop: any) => prop.v;
    const asset = { ...audioAsset(buffer), kind: 'video', name: 'interview.mp4' };
    h.assets.set(asset.id, asset);
    h.PM.proj.layers = [{
      id: 'video-1', name: 'Interview', type: 'video', on: true,
      from: 2, dur: 5,
      d: { asset: asset.id, trim: { v: 1, kf: [], expr: null }, speed: 1, embeddedAudio: true },
    }];

    expect(h.PM.Audio.hasAudibleLayers()).toBe(true);
    h.PM.Audio.start(3);

    expect(h.contexts[0].sources[0].starts[0]).toEqual({ when: 0.25, offset: 2, duration: 4 });
    expect(h.PM.Audio.inspect().voices).toEqual([
      { layerId: 'video-1:embedded-audio', assetId: 'asset-1', sourceOffset: 2, duration: 4 },
    ]);
  });

  it('schedules a future clip at its in-point', () => {
    const buffer = new FakeAudioBuffer([new Float32Array(40)], 4);
    const h = audioHarness();
    const asset = audioAsset(buffer);
    h.assets.set(asset.id, asset);
    h.PM.proj.layers = [audioLayer({ from: 1.5, dur: 4, d: { trim: 1 } })];

    h.PM.Audio.start(1);

    const source = h.contexts[0].sources[0];
    expect(source.starts[0]).toEqual({ when: 0.75, offset: 1, duration: 4 });
    expect(source.connectedTo.gain.values[0].at).toBe(0.75);
  });

  it('does not resurrect playback when a decode finishes after pause', async () => {
    const pending = deferred<FakeAudioBuffer>();
    const buffer = new FakeAudioBuffer([new Float32Array(16)], 4);
    const h = audioHarness({ decode: () => pending.promise });
    const asset = { ...audioAsset(buffer), dur: 4, audioBuffer: null, peaks: null };
    h.assets.set(asset.id, asset);
    h.PM.proj.layers = [audioLayer({ dur: 4 })];

    h.PM.Audio.start(0);
    await flush();
    const generation = h.PM.Audio.inspect().generation;
    expect(h.decodeCalls).toHaveLength(1);
    h.PM.Audio.pause();
    pending.resolve(buffer);
    await flush(2);

    expect(h.PM.Audio.inspect().generation).toBeGreaterThan(generation);
    expect(h.PM.Audio.inspect()).toMatchObject({ running: false, voices: [] });
    expect(h.contexts[0].sources).toHaveLength(0);
    h.PM.Audio.start(1);
    expect(h.contexts[0].sources[0].starts[0]).toEqual({ when: 0.25, offset: 1, duration: 3 });
  });

  it('backs off after lazy decode failure instead of retrying and toasting each tick', async () => {
    const h = audioHarness({ decode: () => { throw new Error('decoder failed'); } });
    const buffer = new FakeAudioBuffer([new Float32Array(16)], 4);
    const asset = { ...audioAsset(buffer), audioBuffer: null, peaks: null, dur: 4 };
    h.assets.set(asset.id, asset);
    h.PM.proj.layers = [audioLayer({ dur: 4 })];

    h.PM.Audio.start(0);
    h.PM.Audio.tick(0.05);
    await flush(2);
    h.PM.Audio.tick(0.1);
    h.PM.Audio.tick(0.15);
    await flush();

    expect(h.decodeCalls).toHaveLength(1);
    expect(h.toasts).toHaveLength(1);
    expect(asset.audioRetryAt).toBeGreaterThan(Date.now());
  });

  it('invalidates an asset decode that is still in flight when disposed', async () => {
    const pending = deferred<FakeAudioBuffer>();
    const buffer = new FakeAudioBuffer([new Float32Array(8)], 4);
    const h = audioHarness({ decode: () => pending.promise });
    const asset = { ...audioAsset(buffer), audioBuffer: null, peaks: null };

    const decoding = h.PM.Audio.decodeAsset(asset);
    await flush();
    h.PM.Audio.disposeAsset(asset);
    pending.resolve(buffer);

    await expect(decoding).resolves.toBeNull();
    expect(asset).toMatchObject({ audioBuffer: null, peaks: null, audioBlob: null });
  });

  it('schedules offline export with the preview offset and fade envelope', async () => {
    const buffer = new FakeAudioBuffer([new Float32Array(40), new Float32Array(40)], 4);
    const h = audioHarness();
    const asset = audioAsset(buffer);
    const layer = audioLayer({ from: 2, dur: 6, d: { trim: 1, gain: 1.5, fadeIn: 3, fadeOut: 2 } });
    h.assets.set(asset.id, asset);
    h.PM.proj.layers = [layer];

    const preview = h.PM.Audio.plan(layer, asset, 1, 6);
    const rendered = await h.PM.Audio.renderOffline(1, 6);
    const offline = h.offlineContexts[0];
    const source = offline.sources[0];
    const gain = source.connectedTo.gain;
    const points = h.PM.Audio.envelopePoints(layer, preview.localStart, preview.duration, preview.audibleDuration);

    expect(rendered).toBe(offline.rendered);
    expect(offline.length).toBe(5 * 48_000);
    expect(source.starts[0]).toEqual({ when: preview.start - 1, offset: preview.sourceOffset, duration: preview.duration });
    expect(gain.values[0]).toEqual({ value: points[0].value, at: preview.start - 1 });
    expect(gain.ramps).toEqual(points.slice(1).map((point: any) => ({
      value: point.value, at: preview.start - 1 + point.local - preview.localStart,
    })));
  });

  it('fails requested audio export visibly for missing and broken tracks', async () => {
    const decoded = new FakeAudioBuffer([new Float32Array(16)], 4);
    const missing = audioHarness({ decodedBuffer: decoded });
    missing.PM.proj.layers = [audioLayer({ name: 'Missing narration', dur: 4 })];
    await expect(missing.PM.Audio.renderOffline(0, 4)).rejects.toThrow(/Audio media is missing for “Missing narration”/);

    const broken = audioHarness({ decode: () => { throw new Error('decoder failed'); } });
    const asset = { ...audioAsset(decoded), audioBuffer: null, peaks: null, dur: 4 };
    broken.assets.set(asset.id, asset);
    broken.PM.proj.layers = [audioLayer({ name: 'Broken narration', dur: 4 })];
    await expect(broken.PM.Audio.renderOffline(0, 4)).rejects.toThrow(/Could not include “Broken narration” in the export/);
  });

  it('pins earlier export tracks during later decode and preserves peaks on eviction', async () => {
    const decoded = new FakeAudioBuffer([new Float32Array(16)], 16);
    const h = audioHarness({ decodedBuffer: decoded });
    const huge = {
      numberOfChannels: 2, length: 50_000_000, sampleRate: 48_000,
      duration: 50_000_000 / 48_000, getChannelData: () => new Float32Array(0),
    };
    const first = { ...audioAsset(decoded, 'asset-1'), audioBuffer: huge, dur: huge.duration, peaks: new Float32Array([0.2, 0.8]) };
    const second = { ...audioAsset(decoded, 'asset-2'), audioBuffer: null, dur: decoded.duration, peaks: null };
    h.assets.set(first.id, first);
    h.assets.set(second.id, second);
    h.PM.proj.layers = [
      audioLayer({ id: 'audio-1', dur: 1, d: { asset: first.id } }),
      audioLayer({ id: 'audio-2', dur: 1, d: { asset: second.id } }),
    ];

    await h.PM.Audio.renderOffline(0, 1);

    const buffers = h.offlineContexts[0].sources.map((source: any) => source.buffer);
    expect(buffers).toContain(huge);
    expect(buffers).toContain(decoded);
    expect(first.audioBuffer).toBeNull();
    expect(Array.from(first.peaks)).toEqual([expect.closeTo(0.2, 5), expect.closeTo(0.8, 5)]);
  });

  it('schedules a realtime export into an isolated stream and cleans it up', async () => {
    const buffer = new FakeAudioBuffer([new Float32Array(40)], 4);
    const h = audioHarness();
    const asset = audioAsset(buffer);
    h.assets.set(asset.id, asset);
    h.PM.proj.layers = [audioLayer({ from: 2, dur: 4, d: { trim: 1, fadeIn: 0.1, fadeOut: 0.2 } })];

    const mix = await h.PM.Audio.createRealtimeMix(1, 6);
    expect(mix.stream.getAudioTracks()).toHaveLength(1);
    expect(mix.start(0.1)).toBe(0.1);
    const source = h.contexts[0].sources[0];
    expect(source.starts[0]).toEqual({ when: 1.35, offset: 1, duration: 4 });
    expect(source.connectedTo.connectedTo.stream).toBe(mix.stream);
    mix.stop();
    expect(source.stopCalls).toBe(1);
    expect(mix.stream.getAudioTracks()[0].stopped).toBe(true);
  });
});
