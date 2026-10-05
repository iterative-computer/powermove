import { describe, expect, it } from 'vitest';

import { Chunker, SAMPLE_RATE, appendAtSeam, findCut, wordsWithin } from './chunking';
import { decodeArgs, decodeError, parseDuration, spanLength } from './audio';

/** Tone with silent gaps at the given seconds (each 0.4 s long). */
function speech(seconds: number, gaps: number[]): Float32Array {
  const samples = new Float32Array(Math.round(seconds * SAMPLE_RATE));
  for (let index = 0; index < samples.length; index++) {
    const time = index / SAMPLE_RATE;
    const silent = gaps.some((gap) => time >= gap && time < gap + 0.4);
    samples[index] = silent ? 0 : 0.3 * Math.sin(index * 0.07);
  }
  return samples;
}

describe('findCut', () => {
  it('cuts in the quietest stretch of the search window', () => {
    const samples = speech(40, [22.3]);
    const cut = findCut(samples, 0, 0, { minSeconds: 18, maxSeconds: 28, overlapSeconds: 1 });
    expect(cut / SAMPLE_RATE).toBeGreaterThan(22.3);
    expect(cut / SAMPLE_RATE).toBeLessThan(22.7);
  });
});

describe('Chunker', () => {
  it('covers the whole stream with owned spans that tile it exactly', () => {
    const chunker = new Chunker({ minSeconds: 18, maxSeconds: 28, overlapSeconds: 1 });
    const audio = speech(95, [20.1, 44.6, 70.2]);
    const windows = [];
    for (let offset = 0; offset < audio.length; offset += 7919) windows.push(...chunker.push(audio.subarray(offset, offset + 7919)));
    const last = chunker.finish();
    if (last) windows.push(last);
    expect(windows.length).toBe(4);
    expect(windows[0]!.ownFrom).toBe(0);
    for (let index = 1; index < windows.length; index++) {
      expect(windows[index]!.ownFrom).toBe(windows[index - 1]!.ownTo);
      // Each window hears a second before its owned span.
      expect(windows[index]!.start / SAMPLE_RATE).toBeCloseTo(windows[index]!.ownFrom - 1, 5);
    }
    expect(windows.at(-1)!.ownTo).toBe(Infinity);
    // Cuts land in the silences.
    expect(windows[0]!.ownTo).toBeGreaterThan(20.1);
    expect(windows[0]!.ownTo).toBeLessThan(20.5);
    // Window samples match the source exactly.
    const second = windows[1]!;
    expect(second.samples[0]).toBe(audio[second.start]);
    expect(chunker.received).toBeCloseTo(95, 3);
  });

  it('hands short media to finish as one window', () => {
    const chunker = new Chunker();
    expect(chunker.push(speech(5, []))).toEqual([]);
    const window = chunker.finish()!;
    expect(window.samples.length).toBe(5 * SAMPLE_RATE);
    expect(window).toMatchObject({ start: 0, ownFrom: 0, ownTo: Infinity });
    expect(chunker.finish()).toBeNull();
  });

  it('keeps each word in exactly one window at a seam', () => {
    const word = (text: string, start: number, end: number) => ({ text, start, end });
    const left = [word('a', 19, 19.5), word('b', 19.8, 20.4), word('c', 20.5, 21)];
    const right = [word('b', 19.8, 20.4), word('c', 20.5, 21), word('d', 21.2, 21.6)];
    expect([...wordsWithin(left, 0, 20.2), ...wordsWithin(right, 20.2, Infinity)].map((w) => w.text)).toEqual(['a', 'b', 'c', 'd']);
  });
});

describe('appendAtSeam', () => {
  const word = (text: string, start: number, end: number) => ({ text, start, end });
  it('drops a seam word both windows kept (seen in a 4-minute smoke test)', () => {
    const kept = [word('wins.', 156.68, 157), word('The', 157.16, 157.24)];
    appendAtSeam(kept, [word('the', 157.27, 157.43), word('best', 157.43, 157.67)]);
    expect(kept.map((w) => w.text)).toEqual(['wins.', 'The', 'best']);
  });
  it('keeps genuinely repeated words that are apart in time', () => {
    const kept = [word('very', 1, 1.3)];
    appendAtSeam(kept, [word('very', 1.5, 1.8), word('good', 1.8, 2.1)]);
    expect(kept.map((w) => w.text)).toEqual(['very', 'very', 'good']);
    const fresh: ReturnType<typeof word>[] = [];
    appendAtSeam(fresh, [word('a', 0, 1)]);
    expect(fresh).toHaveLength(1);
  });
});

describe('ffmpeg decoding', () => {
  it('asks for 16 kHz mono float from the first audio stream and honours the span', () => {
    const args = decodeArgs('/media/clip.mov', { start: 12.5, end: 20 });
    expect(args.join(' ')).toContain('-ss 12.500 -t 7.500 -i /media/clip.mov -map 0:a:0');
    expect(args.slice(-7)).toEqual(['-ac', '1', '-ar', '16000', '-f', 'f32le', 'pipe:1']);
    expect(decodeArgs('/a.wav')).not.toContain('-ss');
  });

  it('reads the duration banner and explains failures', () => {
    expect(parseDuration('  Duration: 01:02:03.50, start: 0.000000')).toBeCloseTo(3723.5);
    expect(parseDuration('nothing')).toBeNull();
    expect(decodeError("Stream map '0:a:0' matches no streams.")).toBe('This file has no audio to transcribe.');
    expect(decodeError('/x: No such file or directory')).toBe('The media file could not be found.');
  });

  it('measures progress against the span that really decodes', () => {
    // An end past the media's end stops where the media does (264 s file, 200 → 9999).
    expect(spanLength(264, 200, 9999)).toBe(64);
    expect(spanLength(264, 200, 230)).toBe(30);
    expect(spanLength(264, 0)).toBe(264);
    expect(spanLength(100, 150)).toBe(0);
  });
});
