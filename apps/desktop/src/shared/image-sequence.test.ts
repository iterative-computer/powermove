import { describe, expect, it } from 'vitest';
import { orderedSequence, sequenceCandidate, validSequenceFps, sequencePlaybackTime } from './image-sequence';

const files = (...names: string[]) => names.map(name => ({ name }));

describe('image sequence ordering', () => {
  it('orders frames numerically with varying padding and a nonzero start', () => {
    expect(orderedSequence(files('shot_10.PNG', 'shot_009.PNG', 'shot_11.PNG')).map(f => f.name))
      .toEqual(['shot_009.PNG', 'shot_10.PNG', 'shot_11.PNG']);
  });
  it('rejects gaps, duplicate frame indices, mixed sequences and unnumbered stills', () => {
    expect(() => orderedSequence(files('f1.png', 'f3.png'))).toThrow('Missing frame 2');
    expect(() => orderedSequence(files('f1.png', 'f01.png'))).toThrow('Duplicate frame');
    for (const names of [['a1.png', 'b2.png'], ['a1.png', 'a2.jpg'], ['a.png', 'b.png'], ['a1.png']]) {
      expect(() => orderedSequence(files(...names))).toThrow();
    }
  });
  it('offers sequences only for matching numbered images and bounds frame rates', () => {
    expect(sequenceCandidate(files('f1.png', 'f2.png'))).toBe(true);
    expect(sequenceCandidate(files('f1.png', 'f2.mp4'))).toBe(false);
    expect(sequenceCandidate(files('f1.png'))).toBe(false);
    for (const fps of [1, 23.976, 29.97, 240]) expect(validSequenceFps(fps)).toBe(true);
    for (const fps of [0, -1, NaN, Infinity, 241]) expect(validSequenceFps(fps)).toBe(false);
  });
});

it('selects the exact source frame at boundaries and retains the final frame at high rates', () => {
  const asset = { imageSequence: { fps: 120, frames: 12 } };
  expect(sequencePlaybackTime(asset, 0)).toBeCloseTo(.001);
  expect(sequencePlaybackTime(asset, 11 / 120)).toBeCloseTo(11 / 120 + .001);
  expect(sequencePlaybackTime(asset, 20)).toBeCloseTo(11 / 120 + .001);
  expect(sequencePlaybackTime(asset, -2)).toBeCloseTo(.001);
  expect(sequencePlaybackTime({}, 1)).toBeUndefined();
});
