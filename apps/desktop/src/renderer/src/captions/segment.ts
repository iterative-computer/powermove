/*
 * Turn timed words into readable caption cues, following common subtitle
 * guidance: at most two lines of ~42 characters, at most ~6 seconds on screen,
 * a new cue after a pause or at the end of a sentence, and a cue never left
 * dangling for a single trailing word when a comma offered a better break.
 */
import { MIN_CUE_DURATION, cueId, normalizeCues, type CaptionCue, type CaptionWord } from './model';

export interface SegmentOptions {
  maxCharsPerLine: number;
  maxLines: number;
  /** Seconds a cue may stay on screen. */
  maxDuration: number;
  /** Seconds of silence that always start a new cue. */
  pause: number;
  /** Seconds a cue lingers after its last word when nothing follows. */
  linger: number;
  /** Shortest cue; short cues are extended into the following gap. */
  minDuration: number;
}

export const DEFAULT_SEGMENT_OPTIONS: SegmentOptions = {
  maxCharsPerLine: 42,
  maxLines: 2,
  maxDuration: 6,
  pause: 0.7,
  linger: 0.4,
  minDuration: 0.8
};

const SENTENCE_END = /[.!?…。！？]["'”’)\]]*$/;
const CLAUSE_END = /[,;:—–、，；：]["'”’)\]]*$/;

const textOf = (words: readonly CaptionWord[]) => words.map(word => word.text).join(' ');

export function segmentWords(input: readonly CaptionWord[], options: Partial<SegmentOptions> = {}, makeId: () => string = cueId): CaptionCue[] {
  const o = { ...DEFAULT_SEGMENT_OPTIONS, ...options };
  const capacity = Math.max(8, o.maxCharsPerLine) * Math.max(1, o.maxLines);
  const words = input
    .map(word => ({ text: String(word.text ?? '').trim(), start: Number(word.start), end: Number(word.end) }))
    .filter(word => word.text && Number.isFinite(word.start) && Number.isFinite(word.end))
    .map(word => word.end < word.start ? { ...word, end: word.start } : word)
    .sort((a, b) => a.start - b.start);
  const groups: CaptionWord[][] = [];
  let current: CaptionWord[] = [];
  const flush = () => { if (current.length) groups.push(current); current = []; };

  for (const word of words) {
    if (current.length) {
      const last = current[current.length - 1]!;
      const gap = word.start - last.end;
      const length = textOf(current).length + 1 + word.text.length;
      const duration = word.end - current[0]!.start;
      if (gap >= o.pause || SENTENCE_END.test(last.text) && textOf(current).length >= 12) flush();
      else if (length > capacity || duration > o.maxDuration) {
        // Prefer the last clause break in the back half of the cue.
        const clause = current.findLastIndex((item, index) => index >= current.length / 2 && CLAUSE_END.test(item.text));
        if (clause >= 0 && clause < current.length - 1) {
          const rest = current.slice(clause + 1);
          current = current.slice(0, clause + 1);
          flush();
          current = rest;
        } else flush();
      }
    }
    current.push(word);
  }
  flush();

  const cues: CaptionCue[] = groups.map(group => ({
    id: makeId(),
    start: group[0]!.start,
    end: Math.max(group[group.length - 1]!.end, group[0]!.start + MIN_CUE_DURATION),
    text: textOf(group),
    words: group.map(word => ({ ...word }))
  }));
  // Linger and minimum duration only ever extend into free time.
  cues.forEach((cue, index) => {
    const next = cues[index + 1];
    const limit = next ? next.start : Infinity;
    const wanted = Math.max(cue.end + o.linger * (next && next.start - cue.end < o.pause ? 0 : 1), cue.start + o.minDuration);
    cue.end = Math.max(cue.end, Math.min(wanted, limit));
  });
  return normalizeCues(cues, makeId);
}
