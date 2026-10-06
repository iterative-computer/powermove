import { describe, expect, it } from 'vitest';
import { segmentWords } from './segment';
import { linearClip, mapWordsToComposition, visibleSourceRange } from './time-map';

const ids = () => { let n = 0; return () => `s${++n}`; };
const words = (text: string, start = 0, step = 0.3, gapAfter: Record<number, number> = {}) => {
  let at = start;
  return text.split(' ').map((token, index) => {
    const word = { text: token, start: at, end: at + step * 0.8 };
    at += step + (gapAfter[index] ?? 0);
    return word;
  });
};

describe('segmentWords', () => {
  it('breaks at sentence ends and long pauses', () => {
    const cues = segmentWords(words('This is the first sentence. And here is another one after a pause', 0, 0.3, { 9: 1.2 }), {}, ids());
    expect(cues.map(cue => cue.text)).toEqual(['This is the first sentence.', 'And here is another one', 'after a pause']);
    expect(cues.every((cue, index) => index === 0 || cue.start >= cues[index - 1]!.end)).toBe(true);
    expect(cues[0]!.words).toHaveLength(5);
  });

  it('respects line capacity and prefers a clause break', () => {
    const text = 'we walked along the river for hours, talking about everything we had ever wanted to do with our lives';
    const cues = segmentWords(words(text, 0, 0.2), { maxCharsPerLine: 32, maxLines: 2 }, ids());
    expect(cues.every(cue => cue.text.length <= 64)).toBe(true);
    expect(cues[0]!.text.endsWith('hours,')).toBe(true);
  });

  it('caps duration and keeps short cues readable', () => {
    const slow = segmentWords(words('one two three four five six seven eight', 0, 1.1), { maxDuration: 3, pause: 5 }, ids());
    expect(slow.every(cue => cue.end - cue.start <= 3.5)).toBe(true);
    const single = segmentWords([{ text: 'Hi', start: 1, end: 1.1 }], {}, ids());
    expect(single[0]!.end - single[0]!.start).toBeCloseTo(0.8);
  });

  it('ignores empty and malformed words', () => {
    expect(segmentWords([{ text: ' ', start: 0, end: 1 }, { text: 'x', start: NaN, end: 1 }])).toEqual([]);
  });
});

describe('source to composition time', () => {
  const transcript = [
    { text: 'a', start: 0, end: 1 },
    { text: 'b', start: 4, end: 5 },
    { text: 'c', start: 10, end: 11 },
    { text: 'd', start: 19.5, end: 20.5 }
  ];

  it('applies start, trim and speed', () => {
    // Clip at 2s in the composition showing source 4s..20s at double speed.
    const clip = linearClip(2, 8, 4, 2);
    expect(visibleSourceRange(clip)).toEqual([4, 20]);
    expect(mapWordsToComposition(transcript, clip)).toEqual([
      { text: 'b', start: 2, end: 2.5 },
      { text: 'c', start: 5, end: 5.5 },
      { text: 'd', start: 9.75, end: 10 }
    ]);
  });

  it('follows a reversed time remap', () => {
    const clip = { from: 0, dur: 10, sourceAt: (t: number) => 10 - t };
    const mapped = mapWordsToComposition(transcript, clip);
    expect(mapped.map(word => word.text)).toEqual(['c', 'b', 'a']);
    expect(mapped[0]!.start).toBeCloseTo(0);
    expect(mapped[1]!.start).toBeCloseTo(5);
    expect(mapped[1]!.end).toBeCloseTo(6);
  });

  it('follows a speed ramp', () => {
    // Half speed for 4s (source 0..2), then double speed.
    const sourceAt = (t: number) => t <= 4 ? t * 0.5 : 2 + (t - 4) * 2;
    const mapped = mapWordsToComposition([{ text: 'x', start: 1, end: 1.5 }, { text: 'y', start: 4, end: 5 }], { from: 0, dur: 8, sourceAt });
    expect(mapped[0]!.start).toBeCloseTo(2, 1);
    expect(mapped[0]!.end).toBeCloseTo(3, 1);
    expect(mapped[1]!.start).toBeCloseTo(5, 1);
    expect(mapped[1]!.end).toBeCloseTo(5.5, 1);
  });
});
