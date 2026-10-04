/*
 * Map transcript times (seconds into the source media) onto the composition.
 *
 * A clip shows its source through `sourceAt(t)`: composition time → source
 * time, the same function playback uses (layer start, trim, constant or
 * keyframed speed, time remapping). The mapping is sampled across the clip
 * and inverted piecewise, so ramps and reversed clips place words correctly.
 * Words the clip never shows are dropped; words cut by the clip edges are
 * clipped to them.
 */
import type { CaptionWord } from './model';

export interface ClipTiming {
  /** Composition time the clip starts. */
  from: number;
  /** Clip duration in composition seconds. */
  dur: number;
  /** Composition time → source time. */
  sourceAt(time: number): number;
  /** Sampling rate in samples per second; defaults to 120. */
  rate?: number;
  /** A constant-rate clip needs one segment, not a sampled curve. */
  linear?: boolean;
}

interface Segment { t0: number; t1: number; s0: number; s1: number }

function segments(clip: ClipTiming): Segment[] {
  const steps = clip.linear ? 1 : Math.max(1, Math.ceil(clip.dur * (clip.rate ?? 120)));
  const out: Segment[] = [];
  let previousT = clip.from, previousS = clip.sourceAt(clip.from);
  for (let index = 1; index <= steps; index++) {
    const t = clip.from + clip.dur * index / steps;
    const s = clip.sourceAt(t);
    out.push({ t0: previousT, t1: t, s0: previousS, s1: s });
    previousT = t; previousS = s;
  }
  return out;
}

/** The source span the clip shows, for transcribing only what is used. */
export function visibleSourceRange(clip: ClipTiming): [number, number] {
  let min = Infinity, max = -Infinity;
  for (const segment of segments(clip)) {
    min = Math.min(min, segment.s0, segment.s1);
    max = Math.max(max, segment.s0, segment.s1);
  }
  return [Math.max(0, min), Math.max(0, max)];
}

const at = (segment: Segment, s: number) => {
  const span = segment.s1 - segment.s0;
  return Math.abs(span) < 1e-9 ? segment.t0 : segment.t0 + (s - segment.s0) / span * (segment.t1 - segment.t0);
};

/** First composition time at which the clip shows source time `s`. Clips
    that only play forward (almost all of them) use a binary search. */
function firstCrossing(list: Segment[], s: number, forward: boolean): number | undefined {
  if (forward) {
    let lo = 0, hi = list.length - 1;
    if (s < list[0]!.s0 || s > list[hi]!.s1) return undefined;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (list[mid]!.s1 < s) lo = mid + 1; else hi = mid;
    }
    return at(list[lo]!, s);
  }
  return crossings(list, s)[0];
}

/** Every composition time at which the clip shows source time `s`. */
function crossings(list: Segment[], s: number): number[] {
  const out: number[] = [];
  for (const segment of list) {
    const lo = Math.min(segment.s0, segment.s1), hi = Math.max(segment.s0, segment.s1);
    if (s < lo || s > hi) continue;
    const t = at(segment, s);
    if (!out.length || Math.abs(out[out.length - 1]! - t) > 1e-6) out.push(t);
  }
  return out;
}

/**
 * Words in composition time, in order. A word that the clip shows more than
 * once (a remap that loops back) is placed at its first appearance.
 */
export function mapWordsToComposition(words: readonly CaptionWord[], clip: ClipTiming): CaptionWord[] {
  const list = segments(clip);
  if (!list.length) return [];
  let visibleMin = Infinity, visibleMax = -Infinity, forward = true;
  for (const segment of list) {
    visibleMin = Math.min(visibleMin, segment.s0, segment.s1);
    visibleMax = Math.max(visibleMax, segment.s0, segment.s1);
    if (segment.s1 < segment.s0) forward = false;
  }
  const clipStart = clip.from, clipEnd = clip.from + clip.dur;
  const out: CaptionWord[] = [];
  for (const word of words) {
    if (word.end < visibleMin || word.start > visibleMax) continue;
    const a = Math.max(word.start, visibleMin), b = Math.min(word.end, visibleMax);
    const ta = firstCrossing(list, a, forward), tb = firstCrossing(list, b, forward);
    if (ta === undefined || tb === undefined) continue;
    const start = Math.max(clipStart, Math.min(ta, tb)), end = Math.min(clipEnd, Math.max(ta, tb));
    if (end < start) continue;
    out.push({ text: word.text, start, end });
  }
  return out.sort((x, y) => x.start - y.start);
}

/** Constant-rate clips, the common case, without sampling. */
export function linearClip(from: number, dur: number, trim: number, speed: number): ClipTiming {
  return { from, dur, linear: true, sourceAt: (time) => (time - from) * speed + trim };
}
