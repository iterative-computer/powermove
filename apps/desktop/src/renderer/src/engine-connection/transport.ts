/*
 * The transport while the new engine draws the composition (plan section
 * 15; contract 6): play, pause, seek, loop, frame steps, J/K/L at 1× and 2×
 * forward and reverse, play around the current frame, and audio scrub go to
 * the interactive engine, and the playhead follows the engine's clock, whose
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
  /** J/K/L: plays from `time` at `rate` (1, 2, −1, −2); a new rate while
   *  playing continues from what is heard. */
  shuttle(time: number, rate: number): void;
  /** Plays from `preRoll` seconds before `time` to `postRoll` after, then
   *  rests on `time` again. */
  playAround(time: number, preRoll: number, postRoll: number): void;
  /** Whether scrubs and steps while paused play a short grain of sound. */
  setAudioScrub(enabled: boolean): void;
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
  private loopValue: { start: number; end: number } | null = null;
  private loopSentOn: DocumentSession | null = null;
  /** A run this driver started: the epoch before it, and whether it played. */
  private run: { before: number | null; played: boolean } | null = null;
  /** Audio scrub as asked, and the session it was sent on. */
  private scrub = false;
  private scrubSentOn: DocumentSession | null = null;

  /** The playhead state asked for while no session could take it (an
   *  exclusive transport during a cold start or a restart): replayed on
   *  the next session that draws. */
  private pending: { playing: boolean; time: number; rate: number } | null = null;
  private pendingAround: { time: number; preRoll: number; postRoll: number } | null = null;
  private live = false;
  /** The playhead as last asked or reported (seconds). */
  private lastTime = 0;

  constructor(
    private readonly session: () => DocumentSession | null,
    private readonly clock: HostClock,
    private readonly options: { exclusive?: boolean; wake?: () => void } = {}
  ) {}

  /** Whether a session draws and takes transport commands now. */
  private drawing(): boolean {
    const s = this.session();
    return !!s && !s.closed && s.route === 'engine' && (s.frames > 0 || s.clock !== null) && s.id !== null;
  }

  active(): boolean {
    if (this.drawing()) return true;
    // "Use New Engine" on: the engine keeps the transport while it starts
    // or restarts, including while there is no mounted viewer.
    return this.options.exclusive === true;
  }

  /** Re-evaluates activity and tells the legacy transport when it changed. */
  refresh(): void {
    const live = this.drawing();
    const came = live && !this.live;
    if (!live && this.live && this.options.exclusive === true && !this.pending) {
      // An engine restart retains the last heard position/rate. The next
      // native session resumes it; no legacy clock or audio takes over.
      const sample = this.session()?.clock;
      const position = this.now(performance.now());
      if (sample && position) this.pending = { playing: position.playing, time: position.time, rate: sample.rate.num / sample.rate.den || 1 };
    }
    this.live = live;
    const active = this.active();
    if (!live && this.pending) this.options.wake?.();
    if (came) { this.syncScrub(); this.syncLoop(); }
    if (came && this.pending) {
      // A session draws again: the playhead goes on from where it was asked.
      const { playing, time, rate } = this.pending;
      this.pending = null;
      const around = this.pendingAround;
      this.pendingAround = null;
      if (playing && around) this.playAround(around.time, around.preRoll, around.postRoll);
      else if (playing) this.shuttle(time, rate);
      else this.seek(time, false);
    }
    if (active === this.wasActive) return;
    this.wasActive = active;
    if (!active) this.run = null;
    else this.syncScrub();
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
    this.shuttle(time, 1);
  }

  /** An exclusive transport asked to move while no session draws: kept
   *  for the next one (`refresh`). Returns whether it was kept. */
  private hold(next: { playing?: boolean; time?: number; rate?: number }): boolean {
    if (this.options.exclusive !== true || this.drawing()) return false;
    const was = this.pending ?? { playing: false, time: this.lastTime, rate: 1 };
    this.pending = { playing: next.playing ?? was.playing, time: next.time ?? was.time, rate: next.rate ?? was.rate };
    this.options.wake?.();
    return true;
  }

  shuttle(time: number, rate: number): void {
    this.pendingAround = null;
    this.lastTime = time;
    if (this.hold({ playing: true, time, rate })) return;
    const s = this.session();
    const clock = s?.clock;
    const moving = !!clock && clock.state !== 'stopped' && clock.rate.num !== 0;
    this.run = { before: clock?.epoch ?? null, played: false };
    // A rate change while the engine plays continues from what is heard.
    if (!moving) this.send({ command: 'seek', time: ticks(time), mode: 'exact' });
    this.send({ command: 'play', rate: { num: rate, den: 1 } });
  }

  playAround(time: number, preRoll: number, postRoll: number): void {
    this.lastTime = time;
    if (this.hold({ playing: true, time, rate: 1 })) {
      this.pendingAround = { time, preRoll, postRoll };
      return;
    }
    const s = this.session();
    this.run = { before: s?.clock?.epoch ?? null, played: false };
    this.send({ command: 'seek', time: ticks(time), mode: 'exact' });
    this.send({ command: 'playAround', preRoll: ticks(preRoll), postRoll: ticks(postRoll) });
  }

  setAudioScrub(enabled: boolean): void {
    this.scrub = enabled;
    this.scrubSentOn = null;
    this.syncScrub();
  }

  /** Sends the audio scrub switch to a session that has not had it. */
  private syncScrub(): void {
    const s = this.session();
    if (!s || s.closed || s.id === null || s === this.scrubSentOn) return;
    this.scrubSentOn = s;
    this.send({ command: 'audioScrub', enabled: this.scrub });
  }

  pause(): void {
    this.pendingAround = null;
    this.run = null;
    if (this.hold({ playing: false })) return;
    this.send({ command: 'pause' });
  }

  seek(time: number, scrub: boolean): void {
    this.lastTime = time;
    if (this.hold({ time })) return;
    if (scrub) this.syncScrub();
    if (this.run) this.run = { before: this.session()?.clock?.epoch ?? null, played: false };
    this.send({ command: 'seek', time: ticks(time), mode: scrub ? 'scrub' : 'exact' });
  }

  setLoop(range: readonly [number, number] | null): void {
    const key = range ? `${range[0]}:${range[1]}` : 'none';
    if (key === this.loop) { this.syncLoop(); return; }
    this.loop = key;
    const loop = range && range[1] > range[0] ? { start: ticks(range[0]), end: ticks(range[1]) } : null;
    this.loopValue = loop && loop.end > loop.start ? loop : null;
    this.loopSentOn = null;
    this.syncLoop();
  }

  private syncLoop(): void {
    const s = this.session();
    if (this.loop === null || !s || !this.drawing() || s.closed || s.id === null || this.loopSentOn === s) return;
    this.loopSentOn = s;
    this.send({ command: 'setLoop', range: this.loopValue });
  }

  /** Forgets the loop sent (a new session starts without one). */
  resetLoop(): void {
    this.loopSentOn = null;
  }

  /** The line's rate: 1, 2, −1, −2 while it moves, 0 when stopped. */
  rate(): number {
    const sample = this.session()?.clock;
    if (!sample || sample.state === 'stopped' || sample.rate.num === 0) return 0;
    return sample.rate.num / sample.rate.den;
  }

  now(nowMs: number): { time: number; playing: boolean; ended: boolean } | null {
    const sample = this.session()?.clock;
    if (!sample) return null;
    const moving = sample.state !== 'stopped' && sample.rate.num !== 0;
    const run = this.run;
    if (run && moving && sample.epoch !== run.before) run.played = true;
    const host = this.clock.hostUs(nowMs);
    if (moving && host === null) return null;
    // A run's first sample is anchored where it starts, at a host time
    // still ahead (the pre-roll, or the audio output's latency): until
    // then the playhead stays where Play put it rather than following the
    // line backwards (contract 6 invariant 3).
    const media = mediaTicksAt(sample, Math.max(host ?? sample.hostTime, sample.hostTime));
    this.lastTime = media / TICKS_PER_SECOND;
    return { time: this.lastTime, playing: moving, ended: !!run?.played && !moving };
  }
}
