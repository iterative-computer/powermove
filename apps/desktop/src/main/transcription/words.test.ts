import { describe, expect, it } from 'vitest';

import { joinWords, removeLag, spreadWords, tokensToWords, wordsToSegments } from './words';

/* Real Parakeet V3 output (sherpa-onnx 1.13.8) for macOS `say`, trimmed. */
const sample = {
  tokens: [' W', 'el', 'c', 'ome', ' to', ' P', 'ow', 'er', 'M', 'ode', '.', ' This', ' is', ' a', ' qu', 'ick', ' test', ' of', ' on', '-', 'dev', 'ice', ' trans', 'cri', 'p', 'tion', '.'],
  timestamps: [0, 0.08, 0.16, 0.24, 0.32, 0.48, 0.64, 0.8, 0.88, 1.04, 1.28, 1.44, 1.6, 1.76, 1.84, 1.92, 2.08, 2.32, 2.48, 2.72, 2.8, 3.04, 3.28, 3.52, 3.6, 3.76, 3.92],
  durations: [0.08, 0.08, 0.08, 0.08, 0.16, 0.16, 0.16, 0.08, 0.16, 0.16, 0.16, 0.16, 0.16, 0.08, 0.08, 0.16, 0.24, 0.16, 0.24, 0.08, 0.08, 0.24, 0.08, 0.08, 0.16, 0.16, 0.16],
  logProbs: [-0.00002, -0.00004, 0, 0, -0.00002, -0.117, -0.0002, 0, -0.33, -0.006, -0.46, 0, 0, 0, -0.001, 0, -0.0001, -0.0001, -0.04, -0.27, -0.04, -0.0002, 0, 0, -0.0003, 0, -0.16]
};

describe('tokensToWords', () => {
  it('joins SentencePiece pieces into words with times and confidence', () => {
    const words = tokensToWords(sample);
    expect(words.map((word) => word.text)).toEqual(['Welcome', 'to', 'PowerMode.', 'This', 'is', 'a', 'quick', 'test', 'of', 'on-device', 'transcription.']);
    expect(words[0]).toMatchObject({ start: 0, end: 0.32 });
    expect(words[2]).toMatchObject({ start: 0.48, end: 1.2 });
    // Punctuation does not drag a word's confidence down.
    expect(words[2]!.confidence).toBeGreaterThan(0.85);
    expect(words[2]!.confidence).toBeLessThan(1);
    for (let index = 1; index < words.length; index++) {
      expect(words[index]!.start).toBeGreaterThanOrEqual(words[index - 1]!.end);
    }
  });

  it('offsets times and clips a word at the next one', () => {
    const words = tokensToWords({ tokens: [' a', ' b'], timestamps: [0, 0.1], durations: [0.4, 0.1] }, 10);
    expect(words).toEqual([{ text: 'a', start: 10, end: 10.1 }, { text: 'b', start: 10.1, end: 10.2 }]);
  });

  it('falls back to a nominal token length without durations and accepts ▁ markers', () => {
    const words = tokensToWords({ tokens: ['▁hi', '▁there'], timestamps: [1, 1.5] });
    expect(words).toEqual([{ text: 'hi', start: 1, end: 1.08 }, { text: 'there', start: 1.5, end: 1.58 }]);
  });
});

describe('wordsToSegments', () => {
  it('ends segments at sentence punctuation', () => {
    const segments = wordsToSegments(tokensToWords(sample));
    expect(segments.map((segment) => segment.text)).toEqual(['Welcome to PowerMode.', 'This is a quick test of on-device transcription.']);
    expect(segments[1]).toMatchObject({ start: 1.44, end: 3.92 });
    expect(segments[1]!.words).toHaveLength(8);
  });

  it('ends segments at long pauses and splits run-ons at a clause', () => {
    const word = (text: string, start: number) => ({ text, start, end: start + 0.3 });
    const paused = wordsToSegments([word('one', 0), word('two', 0.4), word('three', 3)]);
    expect(paused.map((segment) => segment.text)).toEqual(['one two', 'three']);
    const long = Array.from({ length: 40 }, (_, index) => word(index === 19 ? 'nineteen,' : `w${index}`, index * 0.4));
    const split = wordsToSegments(long, { maxDuration: 12 });
    expect(split.length).toBe(2);
    expect(split[0]!.text.endsWith('nineteen,')).toBe(true);
  });

  it('joins CJK characters without spaces', () => {
    expect(joinWords([{ text: '你', start: 0, end: 1 }, { text: '好', start: 1, end: 2 }, { text: 'Powermove', start: 2, end: 3 }])).toBe('你好 Powermove');
  });
});

describe('word boundaries and phrase-timed words', () => {
  it('starts a new word after a token that is only a space (Parakeet numbers)', () => {
    const words = tokensToWords({ tokens: [' of', ' ', '5', '%', ',', ' or', ' ', '2', '0'], timestamps: [0, 0.2, 0.2, 0.3, 0.4, 0.5, 0.7, 0.7, 0.8] });
    expect(words.map((word) => word.text)).toEqual(['of', '5%,', 'or', '20']);
  });

  it('spreads a phrase over its words by length and joins detached punctuation', () => {
    const words = spreadWords('Bonjour à tous , ça va ?', 0, 2.2);
    expect(words.map((word) => word.text)).toEqual(['Bonjour', 'à', 'tous,', 'ça', 'va?']);
    // Weighted by letters + 1: Bonjour takes 8 of 21 parts.
    expect(words[0]).toEqual({ text: 'Bonjour', start: 0, end: 0.838 });
    expect(words.at(-1)!.end).toBe(2.2);
    expect(spreadWords('  ', 0, 1)).toEqual([]);
    expect(spreadWords('…', 0, 1)).toEqual([]);
  });

  it('splits CJK text per character', () => {
    expect(spreadWords('你好，世界。OK', 0, 1).map((word) => word.text)).toEqual(['你', '好，', '世', '界。', 'OK']);
  });

  it('removes a reporting lag without overlapping words', () => {
    const words = removeLag([{ text: 'a', start: 1.3, end: 1.4 }, { text: 'b', start: 1.5, end: 1.6 }, { text: 'c', start: 0.1, end: 0.12 }], 0.3, 0.08);
    expect(words[0]).toEqual({ text: 'a', start: 1, end: 1.32 });
    expect(words[1]).toEqual({ text: 'b', start: 1.32, end: 1.52 });
    expect(words[2]!.end - words[2]!.start).toBeCloseTo(0.02, 5);
  });
});
