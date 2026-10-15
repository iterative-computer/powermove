/*
 * The transport while the new engine draws the composition (plan section
 * 15; contract 6): play, pause, seek, loop, and frame steps go to the
 * interactive engine, and the playhead follows the engine's clock, whose
 * master is the audio device clock. The legacy transport
 * (legacy/core/engine.ts) asks the driver whether it is active and hands
 * over to it, and back, when the composition's route changes.
 */
import type { ClockSample, TransportCommand } from '../../../shared/engine-protocol';
import type { HostClock } from './host-clock';
import type { DocumentSession } from './session';

export const TICKS_PER_SECOND = 705_600_000;

/** What the legacy transport calls (`PM.setTransportDriver`). */
export interface TransportDriver {
  /** Whether the engine runs the transport now. */
  active(): boolean;
  play(time: number): void;
  pause(): void;
  seek(time: number, scrub: boolean): void;
  /** The work-area loop (seconds), or null. */
  setLoop(range: readonly [number, number] | null): void;
  /** The playhead at performance.now() `nowMs`: seconds, whether the line
   *  moves, and whether a run this driver started has ended. */
  now(nowMs: number): { time: number; playing: boolean; ended: boolean } | null;
  onActiveChange(listener: (active: boolean) => void): () => void;
}

export function ticks(seconds: number): number {
  return Math.max(0, Math.round(seconds * TICKS_PER_SECOND));
}

/** Media ticks at host µs `at` on a sample's line (contract 6 invariant 1). */
export function mediaTicksAt(sample: ClockSample, at: number): number {
  const moving = sample.state !== 'stopped' && sample.rate.num !== 0;
  if (!moving) return sample.mediaTime;
  let t = sample.mediaTime + Math.floor(((at - sample.hostTime) * TICKS_PER_SECOND * sample.rate.num) / (1_000_000 * sample.rate.den));
  const loop = sample.loopRange;
  if (loop && loop.end > loop.start) {
    const length = loop.end - loop.start;
    t = loop.start + ((((t - loop.start) % length) + length) % length);
  }
  return t;
}

export class EngineTransport implements TransportDriver {
  private readonly listeners = new Set<(active: boolean) => void>();
  private wasActive = false;
  private loop: string | null = null;
  /** A run this driver started: the epoch before it, and whether it played. */
  private run: { before: number | null; played: boolean } | null = null;

  constructor(
    private readonly session: () => DocumentSession | null,
    private readonly clock: HostClock
  ) {}

  active(): boolean {
    const s = this.session();
    return !!s && !s.closed && s.route === 'engine' && s.frames > 0 && s.id !== null;
  }

  /** Re-evaluates activity and tells the legacy transport when it changed. */
  refresh(): void {
    const active = this.active();
    if (active === this.wasActive) return;
    this.wasActive = active;
    if (!active) this.run = null;
    for (const listener of [...this.listeners]) listener(active);
  }

  onActiveChange(listener: (active: boolean) => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  private send(command: TransportCommand): void {
    this.session()?.transport(command);
  }

  play(time: number): void {
    const s = this.session();
    this.run = { before: s?.clock?.epoch ?? null, played: false };
    this.send({ command: 'seek', time: ticks(time), mode: 'exact' });
    this.send({ command: 'play', rate: { num: 1, den: 1 } });
  }

  pause(): void {
    this.run = null;
    this.send({ command: 'pause' });
  }

  seek(time: number, scrub: boolean): void {
    if (this.run) this.run = { before: this.session()?.clock?.epoch ?? null, played: false };
    this.send({ command: 'seek', time: ticks(time), mode: scrub ? 'scrub' : 'exact' });
  }

  setLoop(range: readonly [number, number] | null): void {
    const key = range ? `${range[0]}:${range[1]}` : 'none';
    if (key === this.loop) return;
    this.loop = key;
    const loop = range && range[1] > range[0] ? { start: ticks(range[0]), end: ticks(range[1]) } : null;
    this.send({ command: 'setLoop', range: loop && loop.end > loop.start ? loop : null });
  }

  /** Forgets the loop sent (a new session starts without one). */
  resetLoop(): void {
    this.loop = null;
  }

  now(nowMs: number): { time: number; playing: boolean; ended: boolean } | null {
    const sample = this.session()?.clock;
    if (!sample) return null;
    const moving = sample.state !== 'stopped' && sample.rate.num !== 0;
    const run = this.run;
    if (run && moving && sample.epoch !== run.before) run.played = true;
    const host = this.clock.hostUs(nowMs);
    if (moving && host === null) return null;
    const media = mediaTicksAt(sample, host ?? sample.hostTime);
    return { time: media / TICKS_PER_SECOND, playing: moving, ended: !!run?.played && !moving };
  }
}
