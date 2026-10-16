/*
 * The viewer's jitter buffer (plan section 18, amendment 4).
 *
 * Timed frames (playback, `duration` set) wait here until the refresh that
 * will show them: at each requestAnimationFrame the viewer predicts when the
 * frame it draws now reaches the display, converts that host instant to
 * media time on the latest clock sample, and draws the frame whose display
 * interval contains it. Frames whose interval already passed are dropped
 * (released undrawn); frames still ahead keep waiting.
 *
 * Frames shown at the next refresh (`duration` null: paused, scrub,
 * gesture) bypass the queue; the newest one wins.
 *
 * Decision phase. A decision exactly on a frame boundary is a coin toss
 * between two frames, and rAF timestamps jitter by about a millisecond.
 * When frames last a whole number of refreshes (60 fps on 120 Hz, 24 fps on
 * 120 Hz), the decision instants are shifted by less than half a refresh so
 * that every boundary falls halfway between two decisions (S1's tuned grid
 * put boundaries half a refresh before vsyncs; here the clock is the audio
 * line, so the viewer moves its decisions instead).
 */
import { mediaTimeAt, running, TICKS_PER_SECOND, type ClockSample, type FrameDescriptor } from './protocol';

export type Queued<T> = { desc: FrameDescriptor; payload: T; arrivedMs: number };

export type Pick<T> = {
  /** Draw this frame now (null: keep what is on screen). */
  show: Queued<T> | null;
  /** Frames to release without drawing, with why. */
  drop: { item: Queued<T>; reason: 'late' | 'superseded' | 'stopped' }[];
};

export class JitterBuffer<T> {
  private timed: Queued<T>[] = [];
  private immediate: Queued<T> | null = null;

  get size(): number {
    return this.timed.length + (this.immediate ? 1 : 0);
  }

  /** The duration (ticks) of the oldest queued timed frame. */
  frameTicks(): number | null {
    return this.timed[0]?.desc.duration ?? null;
  }

  /** Adds an arriving frame; returns frames it supersedes. */
  push(item: Queued<T>): Queued<T>[] {
    if (item.desc.duration === null) {
      const previous = this.immediate;
      if (previous && previous.desc.seq > item.desc.seq) return [item];
      this.immediate = item;
      return previous ? [previous] : [];
    }
    this.timed.push(item);
    return [];
  }

  /** The frame shown at the next refresh, taken out (drawn on arrival instead). */
  takeImmediate(): Queued<T> | null {
    const item = this.immediate;
    this.immediate = null;
    return item;
  }

  /** Every queued frame, emptied (mode changes, teardown, a new pool). */
  clear(filter: (item: Queued<T>) => boolean = () => true): Queued<T>[] {
    const out: Queued<T>[] = [];
    if (this.immediate && filter(this.immediate)) {
      out.push(this.immediate);
      this.immediate = null;
    }
    const keep: Queued<T>[] = [];
    for (const item of this.timed) (filter(item) ? out : keep).push(item);
    this.timed = keep;
    return out;
  }

  /** The decision for the refresh whose frame is predicted to reach the
   *  display at host µs `atUs`. */
  pick(atUs: number, clock: ClockSample | null): Pick<T> {
    const drop: Pick<T>['drop'] = [];
    if (!running(clock)) {
      // Timed frames belong to a line that stopped.
      for (const item of this.timed) drop.push({ item, reason: 'stopped' });
      this.timed = [];
      const show = this.immediate;
      this.immediate = null;
      return { show, drop };
    }
    if (this.immediate) {
      const show = this.immediate;
      this.immediate = null;
      return { show, drop };
    }
    const media = mediaTimeAt(clock!, atUs);
    const loop = clock!.loopRange && clock!.loopRange.end > clock!.loopRange.start ? clock!.loopRange.end - clock!.loopRange.start : null;
    const delta = (pts: number): number => {
      let d = pts - media;
      if (loop !== null) {
        d = ((d % loop) + loop) % loop;
        if (d >= loop / 2) d -= loop;
      }
      return d;
    };
    const forward = clock!.rate.num > 0;
    const contains = (item: Queued<T>): boolean => {
      const d = delta(item.desc.pts);
      return d <= 0 && d > -(item.desc.duration ?? 0);
    };
    const behind = (item: Queued<T>): boolean => {
      const d = delta(item.desc.pts);
      return forward ? d <= -(item.desc.duration ?? 0) : d > 0;
    };
    const index = this.timed.findIndex(contains);
    if (index >= 0) {
      for (const item of this.timed.splice(0, index)) drop.push({ item, reason: 'late' });
      const show = this.timed.shift() ?? null;
      return { show, drop };
    }
    while (this.timed.length && behind(this.timed[0]!)) drop.push({ item: this.timed.shift()!, reason: 'late' });
    return { show: null, drop };
  }
}

/**
 * How far to move decision instants (µs) so frame boundaries fall halfway
 * between them, or 0 when frames do not last a whole number of refreshes.
 *
 * - `periodUs`: the refresh period;
 * - `decisionUs`: any decision instant (host µs) on the refresh grid;
 * - `frameTicks`: a frame's duration in media ticks.
 */
export function decisionOffsetUs(periodUs: number, decisionUs: number, clock: ClockSample | null, frameTicks: number): number {
  if (!running(clock) || !(periodUs > 0) || !(frameTicks > 0)) return 0;
  const rate = clock!.rate.num / clock!.rate.den;
  const frameUs = (frameTicks / TICKS_PER_SECOND) * 1e6 / Math.abs(rate);
  const refreshes = frameUs / periodUs;
  if (Math.round(refreshes) < 1 || Math.abs(refreshes - Math.round(refreshes)) > 0.01) return 0;
  // A boundary near the sample's instant, in host µs.
  const k = Math.round(clock!.mediaTime / frameTicks);
  const boundaryUs = clock!.hostTime + ((k * frameTicks - clock!.mediaTime) / TICKS_PER_SECOND) * 1e6 / rate;
  const phase = ((((boundaryUs - decisionUs) % periodUs) + periodUs) % periodUs);
  let offset = phase - periodUs / 2;
  if (offset <= -periodUs / 2) offset += periodUs;
  return offset;
}
