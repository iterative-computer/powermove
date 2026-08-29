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

describe('legacy audio edge cases', () => {
  it('creates one voice after one shared lazy decode despite repeated demand', async () => {
    const pending = deferred<FakeAudioBuffer>();
    const buffer = new FakeAudioBuffer([new Float32Array(40)], 10);
    const h = audioHarness({ decode: () => pending.promise, currentTime: 10 });
    const lazy = { ...audioAsset(buffer), audioBuffer: null };
    h.assets.set(lazy.id, lazy);
    h.PM.proj.layers = [audioLayer({ dur: 4 })];

    h.PM.Audio.start(0);
    h.PM.Audio.tick(0.05);
    h.PM.Audio.tick(0.1);
    h.PM.Audio.drawWaveform({ fillRect() {} }, h.PM.proj.layers[0], { x: 0, y: 0, width: 100, height: 20 });
    await flush(2);
    expect(h.decodeCalls).toHaveLength(1);

    h.PM.time = 0.2;
    pending.resolve(buffer);
    await flush(6);

    expect(h.contexts[0].sources).toHaveLength(1);
    expect(h.PM.Audio.inspect().voices).toHaveLength(1);
    expect(h.contexts[0].sources[0].starts[0].offset).toBeCloseTo(0.2);
  });

  it('keeps a voice on a normal tick but restarts it for meaningful clock drift', () => {
    const buffer = new FakeAudioBuffer([new Float32Array(100)], 10);
    const h = audioHarness({ decode: () => buffer, currentTime: 10 });
    h.assets.set('asset-1', audioAsset(buffer));
    h.PM.proj.layers = [audioLayer({ dur: 8 })];

    h.PM.Audio.start(1);
    const context = h.contexts[0];
    context.currentTime = 10.5;
    h.PM.Audio.tick(1.5);
    expect(context.sources).toHaveLength(1);

    context.currentTime = 10.6;
    h.PM.Audio.tick(2.1);
    expect(context.sources).toHaveLength(2);
    expect(context.sources[0].stopCalls).toBe(1);
    expect(context.sources[1].starts[0].offset).toBeCloseTo(2.1);
  });

  it('preserves a sub-frame fade and fades at effective source EOF', () => {
    const buffer = new FakeAudioBuffer([new Float32Array(1000)], 1000);
    const h = audioHarness({ decode: () => buffer, currentTime: 10 });
    h.assets.set('asset-1', audioAsset(buffer));
    const layer = audioLayer({ dur: 5, d: { fadeIn: 0.001, fadeOut: 0.25 } });
    h.PM.proj.layers = [layer];

    expect(h.PM.Audio.envelopePoints(layer, 0, 1, 1).map((point: any) => ({
      local: Number(point.local.toFixed(6)), value: Number(point.value.toFixed(6)),
    }))).toEqual([
      { local: 0, value: 0 },
      { local: 0.001, value: 1 },
      { local: 0.75, value: 1 },
      { local: 1, value: 0 },
    ]);

    h.PM.Audio.start(0);
    const gain = h.contexts[0].sources[0].connectedTo.gain;
    expect(gain.curves).toHaveLength(0);
    const events = gain.events.filter((event: any) => event.method !== 'cancel');
    const hasPoint = (method: string, value: number, at: number) => events.some((event: any) =>
      event.method === method && Math.abs(event.value - value) <= 1e-6 && Math.abs(event.at - at) <= 1e-6);
    expect(hasPoint('set', 0, 10)).toBe(true);
    expect(hasPoint('linear', 1, 10.001)).toBe(true);
    expect(events.some((event: any) => event.value === 1 && Math.abs(event.at - 10.75) <= 1e-6)).toBe(true);
    expect(hasPoint('linear', 0, 11)).toBe(true);
  });

  it('discovers nested precomp audio with identical clipped playback and export timing', async () => {
    const buffer = new FakeAudioBuffer([new Float32Array(100)], 10);
    const h = audioHarness({ decode: () => buffer, currentTime: 10 });
    h.assets.set('asset-1', audioAsset(buffer));
    h.PM.proj.layers = [{ id: 'precomp-1', type: 'precomp', on: true, from: 2, dur: 4, d: { comp: 'inner' } }];
    h.PM.proj.comps = { inner: { layers: [audioLayer({ id: 'nested-audio', from: 1, dur: 5, d: { trim: 0.25 } })] } };

    expect(h.PM.Audio.hasAudibleLayers(h.PM.proj)).toBe(true);
    h.PM.Audio.start(3.5);
    expect(h.PM.Audio.inspect().voices[0]).toMatchObject({
      layerId: 'precomp-1/nested-audio', sourceOffset: 0.75, duration: 2.5,
    });

    h.PM.Audio.pause();
    await h.PM.Audio.renderOffline(0, 8);
    expect(h.offlineContexts[0].sources[0].starts[0]).toEqual({ when: 3, offset: 0.25, duration: 3 });
  });

  it('exposes an audio track for recorder fallback and detects deterministic Opus support', async () => {
    const buffer = new FakeAudioBuffer([new Float32Array(40)], 4);
    const h = audioHarness({ currentTime: 10 });
    const asset = audioAsset(buffer);
    h.assets.set(asset.id, asset);
    h.PM.proj.layers = [audioLayer({ from: 2, dur: 4, d: { trim: 1 } })];

    const mix = await h.PM.Audio.createRealtimeMix(1, 6);
    expect(mix.stream.getAudioTracks()).toHaveLength(1);
    expect(mix.stream.getTracks()).toEqual(mix.stream.getAudioTracks());

    h.window.AudioData = class {};
    h.window.AudioEncoder = class {
      static isConfigSupported(config: any) { return Promise.resolve({ supported: config.codec === 'opus' }); }
    };
    await expect(h.PM.Audio.supportsOpus()).resolves.toBe(true);
    mix.stop();
    expect(mix.stream.getAudioTracks()[0].stopped).toBe(true);
  });
});
