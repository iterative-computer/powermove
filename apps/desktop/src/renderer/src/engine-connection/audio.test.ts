/*
 * The engine's audio for today's audio panels: the mixer's meters read the
 * engine's levels while it runs the transport, the monitor solo reaches the
 * engine (and the legacy mix), and waveform peaks fill `asset.peaks`.
 */
import { describe, expect, it, vi } from 'vitest';

import type { AudioMessage } from '../../../shared/engine-protocol';
import { EngineAudio, LEVELS_STALE_MS, MASTER_TAP, routeAudioPanels } from './audio';
import type { DocumentSession } from './session';

function fakeSession() {
  const sent: AudioMessage[] = [];
  const session = {
    id: 5,
    closed: false,
    clock: { state: 'playing' } as unknown,
    audio: (message: AudioMessage) => sent.push(message)
  } as unknown as DocumentSession;
  return { session, sent };
}

function rig(assets: Record<string, any>[] = [], source?: (id: string) => unknown) {
  let now = 1000;
  const { session, sent } = fakeSession();
  const changed = vi.fn();
  let driving = true;
  const audio = new EngineAudio({
    session: () => session,
    driving: () => driving,
    assets: () => assets,
    source,
    peaksChanged: changed,
    now: () => now
  });
  return { audio, session, sent, changed, setNow: (t: number) => (now = t), setDriving: (d: boolean) => (driving = d) };
}

const levels = (peak: number): AudioMessage => ({
  audio: 'levels',
  mediaTime: 0,
  master: { peak: [peak, peak / 2], rms: [peak / 4, peak / 8] },
  strips: [{ id: 'L1', levels: { peak: [peak], rms: [peak / 3] } }]
});

describe('the engine audio for the audio panels', () => {
  it('meters the levels heard, both sides of a mono strip, and silence once they stop', () => {
    const { audio, session, setNow } = rig();
    audio.receive(levels(0.8), session);
    const out = { peak: [0, 0], rms: [0, 0] };
    expect(audio.meters.read(MASTER_TAP, out)).toBe(true);
    expect(out).toEqual({ peak: [0.8, 0.4], rms: [0.2, 0.1] });
    expect(audio.meters.read('L1', out)).toBe(true);
    expect(out.peak).toEqual([0.8, 0.8]);
    expect(out.rms[1]).toBeCloseTo(0.8 / 3, 6);
    expect(audio.meters.read('missing', out)).toBe(false);
    expect(audio.meters.running()).toBe(true);
    setNow(1000 + LEVELS_STALE_MS + 1);
    expect(audio.meters.read(MASTER_TAP, out)).toBe(true);
    expect(out).toEqual({ peak: [0, 0], rms: [0, 0] });
    expect(audio.meters.running()).toBe(false);
  });

  it('sends the monitor solo, asks for the peaks of assets with sound once, and fills asset.peaks', () => {
    const assets = [
      { id: 'a1', kind: 'audio', peaks: null },
      { id: 'v1', kind: 'video' },
      { id: 'v2', kind: 'video', hasAudio: false },
      { id: 'i1', kind: 'image' }
    ];
    const { audio, session, sent, changed } = rig(assets);
    audio.setMonitorSolo(['L1']);
    audio.sync();
    audio.sync();
    expect(sent).toEqual([
      { audio: 'monitor', solo: ['L1'] },
      { audio: 'monitor', solo: ['L1'] },
      { audio: 'peaksWanted', asset: 'a1', request: 1 },
      { audio: 'peaksWanted', asset: 'v1', request: 2 }
    ]);
    audio.receive({ audio: 'peaks', asset: 'a1', request: 1, peaksPerSecond: 100, duration: 0.03, peaks: [0.1, 0.5, 0.2], error: null }, session);
    expect(Array.from(assets[0]!.peaks as unknown as Float32Array)).toEqual([0.1, 0.5, 0.2].map(Math.fround));
    expect(changed).toHaveBeenCalledTimes(1);
    // A failure leaves the asset as it was, and the next sync asks again
    // (offline media resolves later).
    audio.receive({ audio: 'peaks', asset: 'v1', request: 2, peaksPerSecond: 100, duration: 0, peaks: [], error: 'the media is offline' }, session);
    expect((assets[1] as any).peaks).toBeUndefined();
    audio.sync();
    expect(sent.at(-1)).toEqual({ audio: 'peaksWanted', asset: 'v1', request: 3 });
    audio.sync();
    expect(sent.filter((m) => m.audio === 'peaksWanted' && m.asset === 'v1')).toHaveLength(2);
  });

  it('routes PM.Audio.meters and the solo to the engine only while it runs the transport', () => {
    const legacyRead = vi.fn(() => true);
    const legacySolo = vi.fn();
    const pmAudio: Record<string, any> = {
      meters: { MASTER: '$master', read: legacyRead, release: vi.fn(), running: () => true },
      setMonitorSolo: legacySolo
    };
    const { audio, session, sent, setDriving } = rig();
    const undo = routeAudioPanels(pmAudio, audio, () => driving);
    let driving = true;
    audio.receive(levels(0.5), session);
    const out = { peak: [0, 0], rms: [0, 0] };
    expect(pmAudio['meters'].read(MASTER_TAP, out)).toBe(true);
    expect(out.peak[0]).toBe(0.5);
    expect(legacyRead).not.toHaveBeenCalled();
    driving = false;
    setDriving(false);
    pmAudio['meters'].read(MASTER_TAP, out);
    expect(legacyRead).toHaveBeenCalled();
    pmAudio['setMonitorSolo'](['L2']);
    expect(legacySolo).toHaveBeenCalledWith(['L2']);
    expect(sent.at(-1)).toEqual({ audio: 'monitor', solo: ['L2'] });
    undo();
    expect(pmAudio['setMonitorSolo']).toBe(legacySolo);
  });
});


function peak(request: number, value: number): AudioMessage {
  return { audio: 'peaks', asset: 'clip', request, peaksPerSecond: 100, duration: .01, peaks: [value], error: null } as AudioMessage;
}

it('rejects the in-flight waveform from before a document resync', () => {
  const assets = [{ id: 'clip', kind: 'audio' }];
  const { audio, session, sent, changed } = rig(assets);
  audio.sync();
  audio.resynced();
  expect(sent.filter(message => message.audio === 'peaksWanted')).toEqual([
    { audio: 'peaksWanted', asset: 'clip', request: 1 },
    { audio: 'peaksWanted', asset: 'clip', request: 2 },
  ]);
  audio.receive(peak(1, .9), session);
  expect(changed).not.toHaveBeenCalled();
  audio.receive(peak(2, .2), session);
  expect(Array.from((assets[0] as any).peaks)).toEqual([Math.fround(.2)]);
});

it('rejects old and out-of-order answers after replacing a source with the same id', () => {
  const assets = [{ id: 'clip', kind: 'audio', contentHash: 'old' }];
  const { audio, session, sent, changed } = rig(assets);
  audio.sync();
  assets[0] = { id: 'clip', kind: 'audio', contentHash: 'new' };
  audio.sync();
  expect(sent.filter(message => message.audio === 'peaksWanted')).toHaveLength(2);
  audio.receive(peak(1, .9), session);
  expect(changed).not.toHaveBeenCalled();
  audio.receive(peak(2, .2), session);
  audio.receive(peak(1, .9), session);
  expect(Array.from((assets[0] as any).peaks)).toEqual([Math.fround(.2)]);
  expect(changed).toHaveBeenCalledTimes(1);
});

it('invalidates a completed waveform on relink even when the asset object stays live', () => {
  const assets = [{ id: 'clip', kind: 'video', contentHash: 'old' }];
  const { audio, session, sent } = rig(assets);
  audio.sync();
  audio.receive(peak(1, .9), session);
  assets[0]!.contentHash = 'new';
  audio.sync();
  expect((assets[0] as any).peaks).toBeNull();
  expect(sent.filter(message => message.audio === 'peaksWanted')).toHaveLength(2);
  audio.receive(peak(1, .9), session);
  audio.receive(peak(2, .2), session);
  expect(Array.from((assets[0] as any).peaks)).toEqual([Math.fround(.2)]);
});

it('rejects answers for assets removed while the decoder was working', () => {
  const assets = [{ id: 'clip', kind: 'audio' }];
  const { audio, session, changed } = rig(assets);
  audio.sync();
  assets.length = 0;
  audio.receive(peak(1, .9), session);
  expect(changed).not.toHaveBeenCalled();
});

it('invalidates a hashless restored asset when its persisted source is relinked', () => {
  const assets: Record<string, any>[] = [{ id: 'clip', kind: 'audio', storageKey: 'sampled-key' }];
  let hash = 'original';
  const { audio, session, sent } = rig(assets, () => [hash]);
  audio.sync();
  audio.receive(peak(1, .9), session);
  hash = 'replacement';
  audio.sync();
  expect(assets[0]!.peaks).toBeNull();
  expect(sent.filter(message => message.audio === 'peaksWanted')).toHaveLength(2);
  audio.receive(peak(1, .9), session);
  expect(assets[0]!.peaks).toBeNull();
  audio.receive(peak(2, .2), session);
  expect(Array.from(assets[0]!.peaks)).toEqual([Math.fround(.2)]);
});

it('does not let failing assets trigger each other into an automatic retry loop', () => {
  const { audio, session, sent } = rig([{ id: 'a', kind: 'audio' }, { id: 'b', kind: 'video' }]);
  audio.sync();
  for (const [asset, request] of [['a', 1], ['b', 2]] as const) {
    audio.receive({ audio: 'peaks', asset, request, peaksPerSecond: 100, duration: 0, peaks: [], error: 'the media is offline' }, session);
  }
  expect(sent.filter(message => message.audio === 'peaksWanted')).toHaveLength(2);
  // A real asset/route sync still retries each source once.
  audio.sync();
  expect(sent.filter(message => message.audio === 'peaksWanted')).toHaveLength(4);
  audio.receive({ audio: 'peaks', asset: 'a', request: 3, peaksPerSecond: 100, duration: 0, peaks: [], error: 'no audio stream' }, session);
  audio.receive({ audio: 'peaks', asset: 'b', request: 4, peaksPerSecond: 100, duration: 0, peaks: [], error: 'no audio stream' }, session);
  expect(sent.filter(message => message.audio === 'peaksWanted')).toHaveLength(4);
});
