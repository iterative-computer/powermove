import type { TranscriptWord } from '../../shared/transcription';

/*
 * Long media is decoded in windows of about half a minute. Each window ends
 * at the quietest moment in its last stretch (a breath between sentences,
 * usually), and neighbouring windows overlap by a second on each side of
 * that cut so a word the cut lands on is heard whole by one of them. Words
 * are then kept from whichever window owns their midpoint: everything before
 * the cut from the left window, everything after from the right.
 */

export const SAMPLE_RATE = 16_000;

export interface ChunkingOptions {
  /** Shortest window before a cut is considered, seconds. */
  minSeconds: number;
  /** Longest window; the cut is found in [min, max]. */
  maxSeconds: number;
  /** Audio shared with each neighbour beyond the cut, seconds. */
  overlapSeconds: number;
}

export const DEFAULT_CHUNKING: ChunkingOptions = { minSeconds: 18, maxSeconds: 28, overlapSeconds: 1 };

const FRAME = SAMPLE_RATE / 100; // 10 ms
const SMOOTH_FRAMES = 20; // judge quietness over 200 ms, not a single stop consonant

/**
 * Where to cut, as an absolute sample index, given samples starting at
 * `bufferStart`. Looks in [from + min, from + max] and returns the middle of
 * the quietest 200 ms there.
 */
export function findCut(samples: Float32Array, bufferStart: number, from: number, options: ChunkingOptions = DEFAULT_CHUNKING): number {
  const lo = Math.max(from + Math.round(options.minSeconds * SAMPLE_RATE), bufferStart);
  const hi = Math.min(from + Math.round(options.maxSeconds * SAMPLE_RATE), bufferStart + samples.length);
  const frames = Math.floor((hi - lo) / FRAME);
  if (frames <= SMOOTH_FRAMES) return Math.min(hi, Math.max(lo, from + Math.round(options.maxSeconds * SAMPLE_RATE)));
  const energy = new Float64Array(frames);
  for (let frame = 0; frame < frames; frame++) {
    let sum = 0;
    const begin = lo - bufferStart + frame * FRAME;
    for (let index = begin; index < begin + FRAME; index++) {
      const value = samples[index] ?? 0;
      sum += value * value;
    }
    energy[frame] = sum;
  }
  let window = 0;
  for (let frame = 0; frame < SMOOTH_FRAMES; frame++) window += energy[frame]!;
  let best = 0;
  let bestEnergy = window;
  for (let frame = SMOOTH_FRAMES; frame < frames; frame++) {
    window += energy[frame]! - energy[frame - SMOOTH_FRAMES]!;
    if (window < bestEnergy) {
      bestEnergy = window;
      best = frame - SMOOTH_FRAMES + 1;
    }
  }
  return lo + (best + SMOOTH_FRAMES / 2) * FRAME;
}

/** Silence fed ahead of every window: Parakeet drops whole phrases when its
    audio starts abruptly (a span or window edge inside speech). A whole
    number of the encoder's 80 ms frames, so word times do not shift. */
export const LEAD_IN_SECONDS = 0.48;

export function withLeadIn(samples: Float32Array, seconds = LEAD_IN_SECONDS): Float32Array {
  const pad = Math.round(seconds * SAMPLE_RATE);
  const out = new Float32Array(pad + samples.length);
  out.set(samples, pad);
  return out;
}

/** Keeps the words whose midpoint lies in [from, to) (seconds, absolute). */
export function wordsWithin(words: TranscriptWord[], from: number, to: number): TranscriptWord[] {
  return words.filter((word) => {
    const middle = (word.start + word.end) / 2;
    return middle >= from && middle < to;
  });
}

const normal = (text: string) => text.toLowerCase().replace(/[\p{P}\p{S}]/gu, '');

/**
 * Appends the next window's words, dropping any the previous window already
 * heard: the two windows time a seam word slightly differently, so its
 * midpoint can land on both sides of the cut.
 */
export function appendAtSeam(kept: TranscriptWord[], incoming: TranscriptWord[]): void {
  let index = 0;
  const last = kept[kept.length - 1];
  while (last && index < incoming.length) {
    const word = incoming[index]!;
    const sameSound = word.start < (last.start + last.end) / 2;
    const repeated = normal(word.text) === normal(last.text) && Math.abs(word.start - last.start) < 0.4;
    if (!sameSound && !repeated) break;
    index++;
  }
  for (; index < incoming.length; index++) kept.push(incoming[index]!);
}

/**
 * Collects 16 kHz samples as they stream in and hands out overlapping
 * windows. `push` returns windows ready to decode; `finish` returns the rest.
 * Each window says which slice of time it owns, for `wordsWithin`.
 */
export interface AudioWindow {
  samples: Float32Array;
  /** Absolute sample index of samples[0]. */
  start: number;
  /** Seconds (absolute, from the decoded start) this window's words own. */
  ownFrom: number;
  ownTo: number;
}

export class Chunker {
  private buffer = new Float32Array(SAMPLE_RATE * 8);
  private length = 0;
  /** Absolute sample index of buffer[0]. */
  private bufferStart = 0;
  /** Absolute sample index where the next window's owned time begins. */
  private from = 0;

  constructor(private readonly options: ChunkingOptions = DEFAULT_CHUNKING) {}

  /** Seconds of audio received so far. */
  get received(): number { return (this.bufferStart + this.length) / SAMPLE_RATE; }

  push(samples: Float32Array): AudioWindow[] {
    this.append(samples);
    const windows: AudioWindow[] = [];
    const overlap = Math.round(this.options.overlapSeconds * SAMPLE_RATE);
    const ready = () => this.bufferStart + this.length >= this.from + Math.round(this.options.maxSeconds * SAMPLE_RATE) + overlap;
    while (ready()) {
      const view = this.buffer.subarray(0, this.length);
      const cut = findCut(view, this.bufferStart, this.from, this.options);
      const start = Math.max(this.bufferStart, this.from - overlap);
      const end = cut + overlap;
      windows.push({
        samples: this.buffer.slice(start - this.bufferStart, end - this.bufferStart),
        start,
        ownFrom: this.from / SAMPLE_RATE,
        ownTo: cut / SAMPLE_RATE
      });
      this.from = cut;
      this.drop(cut - overlap);
    }
    return windows;
  }

  finish(): AudioWindow | null {
    const total = this.bufferStart + this.length;
    if (total <= this.from) return null;
    const overlap = Math.round(this.options.overlapSeconds * SAMPLE_RATE);
    const start = Math.max(this.bufferStart, this.from - overlap);
    const window = {
      samples: this.buffer.slice(start - this.bufferStart, this.length),
      start,
      ownFrom: this.from / SAMPLE_RATE,
      ownTo: Infinity
    };
    this.from = total;
    return window;
  }

  private append(samples: Float32Array): void {
    if (this.length + samples.length > this.buffer.length) {
      const next = new Float32Array(Math.max(this.buffer.length * 2, this.length + samples.length));
      next.set(this.buffer.subarray(0, this.length));
      this.buffer = next;
    }
    this.buffer.set(samples, this.length);
    this.length += samples.length;
  }

  /** Forgets samples before absolute index `keepFrom`. */
  private drop(keepFrom: number): void {
    const count = Math.max(0, Math.min(this.length, keepFrom - this.bufferStart));
    if (!count) return;
    this.buffer.copyWithin(0, count, this.length);
    this.length -= count;
    this.bufferStart += count;
  }
}
