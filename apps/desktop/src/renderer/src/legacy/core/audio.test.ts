import { describe, expect, it, vi } from 'vitest';

import type { PMRegistry } from '../registry';
import { install } from './audio';

function audioRegistry({ host = {}, layers = [] }: any = {}): PMRegistry {
  vi.stubGlobal('window', host);
  const listeners = new Map<string, Function[]>();
  const PM: PMRegistry = {
    proj: { dur: 12, fps: 30, work: [0, 12], layers, assets: {} },
    assets: { map: new Map(), get() { return null; } },
    time: 0,
    playing: false,
    bus: {
      on(event: string, handler: Function) { listeners.set(event, [...(listeners.get(event) || []), handler]); },
      emit(event: string) { for (const handler of listeners.get(event) || []) handler(); },
    },
    invalidate() {},
    toast() {},
  };
  install(PM);
  return PM;
}

function audioLayer(): any {
  return {
    id: 'audio-1', name: 'Audio', type: 'audio', on: true,
    from: 0, dur: 4, d: { asset: 'asset-1', trim: 0, gain: 2, fadeIn: 3, fadeOut: 3 },
  };
}

describe('legacy audio install', () => {
  it('hands the decoder its owned input without allocating a second full audio buffer', async () => {
    const bytes = new ArrayBuffer(64);
    const decoded = { duration: 1, length: 4, sampleRate: 4, numberOfChannels: 1, getChannelData: () => new Float32Array([0, .5, -.5, 0]) };
    const decodeAudioData = vi.fn(async (_bytes: ArrayBuffer) => decoded);
    const PM = audioRegistry({ host: { AudioContext: class {
      destination = {};
      createGain() { return { gain: { value: 0 }, connect() {} }; }
      decodeAudioData = decodeAudioData;
    } } });
    const asset = await PM.Audio.prepareAsset({ id: 'audio', name: 'tone.wav', blob: { size: 64, arrayBuffer: async () => bytes } });
    expect(decodeAudioData.mock.calls[0]?.[0]).toBe(bytes);
    expect(asset.audioBuffer).toBe(decoded);
    expect([...asset.peaks]).toEqual([0, .5, .5, 0]);
  });
  function idleAudio() {
    const callbacks: Function[] = [];
    const resume = vi.fn();
    const createContext = vi.fn(function () {
      return { state: 'suspended', createGain: () => ({ gain: { value: 0 }, connect() {} }), destination: {}, close: vi.fn(), resume };
    });
    const host = {
      AudioContext: createContext,
      requestIdleCallback: vi.fn((callback: Function) => { callbacks.push(callback); return callbacks.length; }),
      cancelIdleCallback: vi.fn(),
    };
    return { callbacks, resume, createContext, host };
  }

  it('prepares an audible project during idle without starting or resuming playback', () => {
    const f = idleAudio();
    const PM = audioRegistry({ host: f.host, layers: [audioLayer()] });
    PM.bus.emit('assets'); PM.bus.emit('layers');
    expect(f.host.requestIdleCallback).toHaveBeenCalledTimes(1);
    expect(f.createContext).not.toHaveBeenCalled();
    f.callbacks.shift()!();
    expect(f.createContext).toHaveBeenCalledTimes(1);
    expect(f.resume).not.toHaveBeenCalled();
    expect(PM.Audio.inspect().running).toBe(false);
    PM.bus.emit('assets');
    expect(f.host.requestIdleCallback).toHaveBeenCalledTimes(1);
  });

  it('does not prepare audio for a silent project or after audio ownership changes', () => {
    const f = idleAudio();
    const PM = audioRegistry({ host: f.host });
    expect(f.host.requestIdleCallback).not.toHaveBeenCalled();
    PM.proj.layers.push(audioLayer()); PM.bus.emit('layers');
    PM.proj.layers = []; PM.bus.emit('layers');
    f.callbacks.shift()!();
    expect(f.createContext).not.toHaveBeenCalled();
    PM.proj.layers.push(audioLayer()); PM.bus.emit('layers');
    PM.Audio.destroy();
    expect(f.host.cancelIdleCallback).toHaveBeenCalled();
    f.callbacks.shift()!();
    expect(f.createContext).not.toHaveBeenCalled();
  });

  it('uses MIME first and a deliberate extension fallback', () => {
    const PM = audioRegistry();

    expect(PM.Audio.accepts({ name: 'recording.bin', type: 'audio/wav' })).toBe(true);
    expect(PM.Audio.accepts({ name: 'renamed.MP3', type: 'application/octet-stream' })).toBe(true);
    expect(PM.Audio.accepts({ name: 'movie.mp4', type: 'video/mp4' })).toBe(false);
  });

  it('keeps overlapping gain and fade envelopes deterministic', () => {
    const PM = audioRegistry();
    const layer = audioLayer();

    expect(PM.Audio.gainAt(layer, 0)).toBe(0);
    expect(PM.Audio.gainAt(layer, 2)).toBeCloseTo(4 / 3);
    expect(PM.Audio.gainAt(layer, 4)).toBe(0);
  });
});
