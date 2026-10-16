/*
 * The engine's audio for today's audio panels (plan section 16; appendix C:
 * "Timeline waveforms; mixer meters, solo, retune | WebAudio | Engine"),
 * behind "Use New Engine".
 *
 * While the engine runs the transport, the legacy Web Audio graph rests, so
 * the mixer panel's meters read the engine's levels (contract 6
 * `AudioMessage` `levels`, about 30 a second while it plays) through the
 * same `PM.Audio.meters` interface, and its monitor solo reaches the
 * engine's mixer as well as the legacy one. Waveform peaks come from the
 * engine's streaming decode (any length; today's whole-file decode stops at
 * its 128 MiB budget) and fill `asset.peaks`, the envelope the timeline,
 * the assets panel, and `api.media.audio.drawWaveform` already draw.
 */
import type { AudioLevels, AudioMessage, AudioPeaks, ChannelLevels } from '../../../shared/engine-protocol';
import type { DocumentSession } from './session';

/** The master tap id of `PM.Audio.meters` (legacy/core/audio.ts). */
export const MASTER_TAP = '$master';

/** Levels older than this read as silence (the engine stopped sending). */
export const LEVELS_STALE_MS = 250;

type Out = { peak: Float32Array | number[]; rms: Float32Array | number[] };

export type LevelSource = {
  readonly MASTER: string;
  read(id: string, out: Out): boolean;
  release(): void;
  running(): boolean;
};

type AssetLike = { id?: string; type?: string; kind?: string; peaks?: ArrayLike<number> | null; hasAudio?: boolean; peaksSource?: string; audioDecodeError?: unknown };

export type EngineAudioHost = {
  /** The session the transport follows. */
  session(): DocumentSession | null;
  /** Whether the engine runs the transport now. */
  driving(): boolean;
  /** The project's assets (`PM.assets.map`). */
  assets(): Iterable<AssetLike>;
  /** An asset's peaks changed: redraw what shows them. */
  peaksChanged(asset: AssetLike): void;
  now?(): number;
};

function copy(levels: ChannelLevels, out: Out): void {
  // Mono buses meter on both sides, as the legacy taps up-mix to two.
  for (let channel = 0; channel < 2; channel++) {
    const c = Math.min(channel, levels.peak.length - 1);
    out.peak[channel] = c >= 0 ? levels.peak[c] ?? 0 : 0;
    out.rms[channel] = c >= 0 ? levels.rms[c] ?? 0 : 0;
  }
}

export class EngineAudio {
  private levels: AudioLevels | null = null;
  private arrived = -Infinity;
  /** Levels messages received (diagnostics: a scrub grain sends a few). */
  levelsReceived = 0;
  private levelListeners = new Set<(levels: AudioLevels) => void>();
  private solo: string[] = [];
  /** The session the solo and the peak requests were sent on. */
  private sentOn: DocumentSession | null = null;
  /** Assets whose peaks were asked for on `sentOn`, or answered. */
  private asked = new Set<string>();
  private answered = new Map<string, AudioPeaks>();

  constructor(private readonly host: EngineAudioHost) {}

  private now(): number {
    return this.host.now?.() ?? performance.now();
  }

  /** An audio message from the engine. */
  receive(message: AudioMessage, session: DocumentSession): void {
    if (session !== this.host.session()) return;
    switch (message.audio) {
      case 'levels': {
        const { audio: _a, ...levels } = message;
        this.levels = levels as AudioLevels;
        this.arrived = this.now();
        this.levelsReceived++;
        for (const listener of this.levelListeners) listener(this.levels);
        return;
      }
      case 'peaks': {
        const { audio: _a, ...peaks } = message;
        if (peaks.error) {
          // Offline media (not resolved yet) or no sound: ask again at the
          // next sync (a route change, an asset change), not at once.
          this.asked.delete(peaks.asset);
          return;
        }
        this.answered.set(peaks.asset, peaks as AudioPeaks);
        this.apply(peaks as AudioPeaks);
        return;
      }
      default:
        return;
    }
  }

  private apply(peaks: AudioPeaks): void {
    if (peaks.error || !peaks.peaks.length) return;
    for (const asset of this.host.assets()) {
      if (asset?.id !== peaks.asset) continue;
      asset.peaks = Float32Array.from(peaks.peaks);
      asset.peaksSource = 'engine';
      this.host.peaksChanged(asset);
    }
  }

  /** The session the transport follows changed or became ready: sends the
   *  monitor solo and asks for the peaks of every asset with sound. */
  sync(): void {
    const session = this.host.session();
    if (!session || session.closed || session.id === null) return;
    if (session !== this.sentOn) {
      this.sentOn = session;
      this.asked.clear();
      this.levels = null;
      session.audio({ audio: 'monitor', solo: this.solo });
    }
    for (const asset of this.host.assets()) {
      const id = asset?.id;
      if (!id || this.asked.has(id) || !hasSound(asset)) continue;
      this.asked.add(id);
      const known = this.answered.get(id);
      if (known && !known.error) {
        this.apply(known);
        continue;
      }
      session.audio({ audio: 'peaksWanted', asset: id });
    }
  }

  /** Every levels message (the measurements and tests). */
  onLevels(listener: (levels: AudioLevels) => void): () => void {
    this.levelListeners.add(listener);
    return () => this.levelListeners.delete(listener);
  }

  /** The newest levels and when they arrived (diagnostics and tests). */
  latest(): { levels: AudioLevels; arrivedMs: number } | null {
    return this.levels ? { levels: this.levels, arrivedMs: this.arrived } : null;
  }

  /** An asset's file changed (relink, replace): ask again. */
  forget(asset: string): void {
    this.asked.delete(asset);
    this.answered.delete(asset);
  }

  setMonitorSolo(ids: Iterable<string> | null | undefined): void {
    this.solo = [...(ids ?? [])].filter((id) => typeof id === 'string' && id);
    const session = this.host.session();
    if (session && !session.closed && session.id !== null) session.audio({ audio: 'monitor', solo: this.solo });
  }

  /** `PM.Audio.meters` while the engine runs the transport. */
  readonly meters: LevelSource = {
    MASTER: MASTER_TAP,
    read: (id, out) => {
      const levels = this.levels;
      if (!levels || this.now() - this.arrived > LEVELS_STALE_MS) {
        out.peak[0] = out.peak[1] = 0;
        out.rms[0] = out.rms[1] = 0;
        return !!levels;
      }
      if (id === MASTER_TAP) {
        copy(levels.master, out);
        return true;
      }
      const strip = levels.strips.find((s) => s.id === id);
      if (!strip) return false;
      copy(strip.levels, out);
      return true;
    },
    release: () => {},
    running: () => {
      const clock = this.host.session()?.clock;
      return !!clock && clock.state !== 'stopped' && this.now() - this.arrived <= LEVELS_STALE_MS;
    }
  };
}

function hasSound(asset: AssetLike): boolean {
  const kind = String(asset.kind ?? asset.type ?? '');
  if (kind === 'audio') return true;
  if (kind === 'video') return asset.hasAudio !== false;
  return false;
}

/**
 * Routes `PM.Audio.meters` and `setMonitorSolo` to the engine while it runs
 * the transport; the legacy implementations keep working otherwise (and
 * keep the solo, so a hand-over back to them sounds the same).
 */
export function routeAudioPanels(audio: Record<string, any> | undefined, engine: EngineAudio, driving: () => boolean): () => void {
  if (!audio) return () => {};
  const legacyMeters = audio['meters'];
  const legacySolo = audio['setMonitorSolo'];
  const meters: LevelSource = {
    get MASTER() { return MASTER_TAP; },
    read: (id, out) => (driving() ? engine.meters.read(id, out) : !!legacyMeters?.read?.(id, out)),
    release: () => {
      engine.meters.release();
      legacyMeters?.release?.();
    },
    running: () => (driving() ? engine.meters.running() : !!legacyMeters?.running?.())
  };
  audio['meters'] = meters;
  audio['setMonitorSolo'] = (ids: Iterable<string> | null | undefined) => {
    const list = [...(ids ?? [])];
    legacySolo?.call(audio, list);
    engine.setMonitorSolo(list);
  };
  return () => {
    audio['meters'] = legacyMeters;
    audio['setMonitorSolo'] = legacySolo;
  };
}
