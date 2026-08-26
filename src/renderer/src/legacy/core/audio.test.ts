import { describe, expect, it, vi } from 'vitest';

import type { PMRegistry } from '../registry';
import { install } from './audio';

function audioRegistry(): PMRegistry {
  vi.stubGlobal('window', {});
  const PM: PMRegistry = {
    proj: { dur: 12, fps: 30, work: [0, 12], layers: [], assets: {} },
    assets: { map: new Map(), get() { return null; } },
    time: 0,
    playing: false,
    bus: { on() {}, emit() {} },
    invalidate() {},
    toast() {},
  };
  install(PM);
  return PM;
}

function audioLayer(): any {
  return {
    id: 'audio-1', name: 'Audio', type: 'audio', on: true, solo: false,
    from: 0, dur: 4, d: { asset: 'asset-1', trim: 0, gain: 2, fadeIn: 3, fadeOut: 3 },
  };
}

describe('legacy audio install', () => {
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
