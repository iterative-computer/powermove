import { describe, expect, it } from 'vitest';

import { windowWords, type EngineResult } from './decode';

const window = { offset: 9.52, soundFrom: 10.2, soundTo: 13.8 };

describe('windowWords', () => {
  it('turns token timestamps into words in source time, minus the model\'s lag', () => {
    const result: EngineResult = {
      text: 'Hello 20 people.',
      tokens: [
        { text: ' Hel', t0Ms: 800, t1Ms: 880, p: 0.99 }, { text: 'lo', t0Ms: 880, t1Ms: 960, p: 0.98 },
        { text: ' ', t0Ms: 1200, t1Ms: 1280, p: 1 }, { text: '2', t0Ms: 1200, t1Ms: 1280, p: 1 }, { text: '0', t0Ms: 1280, t1Ms: 1360, p: 1 },
        { text: ' people', t0Ms: 1520, t1Ms: 1680, p: 0.9 }, { text: '.', t0Ms: 1840, t1Ms: 1920, p: 0.8 }
      ],
      segments: []
    };
    const words = windowWords(result, 'word', window);
    expect(words.map((word) => word.text)).toEqual(['Hello', '20', 'people.']);
    expect(words[0]).toMatchObject({ start: 10.32, end: 10.48 });
    expect(words[1]).toMatchObject({ start: 10.72, end: 10.88 });
    expect(words[0]!.confidence).toBeCloseTo(Math.sqrt(0.99 * 0.98), 3);
    const late = windowWords(result, 'word', window, [0.3, 0.2, 0.08]);
    expect(late[0]).toMatchObject({ start: 10.02, end: 10.4 });
    // “20” follows a 240 ms gap: an onset, 0.3 back. Never before the previous word ends.
    expect(late[1]).toMatchObject({ start: 10.42 });
    expect(late[1]!.start).toBeGreaterThanOrEqual(late[0]!.end);
  });

  it('spreads phrase-timed words over their phrase, inside the window\'s sound', () => {
    const result: EngineResult = {
      text: 'Guten Tag. Wie geht es?',
      tokens: [],
      segments: [{ text: ' Guten Tag.', t0Ms: 0, t1Ms: 1600, p: 1 } as never, { text: ' Wie geht es?', t0Ms: 2000, t1Ms: 3200 } as never]
    };
    const words = windowWords(result, 'segment', window);
    expect(words.map((word) => word.text)).toEqual(['Guten', 'Tag.', 'Wie', 'geht', 'es?']);
    // The first phrase starts in the lead-in: it is clamped to where sound begins.
    expect(words[0]!.start).toBe(10.2);
    expect(words[1]!.end).toBe(11.12);
    expect(words[2]!.start).toBe(11.52);
    expect(words[4]!.end).toBe(12.72);
  });

  it('spreads a text-only model\'s words over the window\'s sound', () => {
    const result: EngineResult = { text: 'heute ist das Wetter warm .', tokens: [], segments: [{ text: '', t0Ms: 0, t1Ms: 0 }] };
    const words = windowWords(result, 'none', window);
    expect(words.map((word) => word.text)).toEqual(['heute', 'ist', 'das', 'Wetter', 'warm.']);
    expect(words[0]!.start).toBe(10.2);
    expect(words.at(-1)!.end).toBe(13.8);
    for (let index = 1; index < words.length; index++) expect(words[index]!.start).toBe(words[index - 1]!.end);
  });

  it('falls back to the window when a phrase-timed model gives no usable phrases', () => {
    const words = windowWords({ text: 'Hello there', tokens: [], segments: [{ text: 'Hello there', t0Ms: 0, t1Ms: 0 }] }, 'segment', window);
    expect(words).toEqual([{ text: 'Hello', start: 10.2, end: 12 }, { text: 'there', start: 12, end: 13.8 }]);
  });
});
