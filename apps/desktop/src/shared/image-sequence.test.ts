import { describe, expect, it } from 'vitest';
import { orderedSequence, sequenceCandidate, sequenceGaps, validSequenceFps, sequencePlaybackTime, sequenceStreamTime } from './image-sequence';

const files = (...names: string[]) => names.map(name => ({ name }));

describe('image sequence ordering', () => {
  it('orders frames numerically with varying padding and a nonzero start', () => {
    expect(orderedSequence(files('shot_10.PNG', 'shot_009.PNG', 'shot_11.PNG')).map(f => f.name))
      .toEqual(['shot_009.PNG', 'shot_10.PNG', 'shot_11.PNG']);
  });
  it('accepts sparse frames and reports compact missing ranges in numeric order', () => {
    const sparse = files('f9.png', 'f1.png', 'f3.png', 'f1000000000.png');
    expect(orderedSequence(sparse).map(f => f.name)).toEqual(['f1.png', 'f3.png', 'f9.png', 'f1000000000.png']);
    expect(sequenceGaps(sparse)).toEqual([{ start: 2, end: 2 }, { start: 4, end: 8 }, { start: 10, end: 999999999 }]);
    expect(sequenceGaps(files('f9.png', 'f10.png'))).toEqual([]);
  });
  it('rejects duplicate frame indices, mixed sequences and unnumbered stills', () => {
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

it('aims a playing decoder at the middle of the frame, absorbing start-up lag either way', () => {
  const asset = { imageSequence: { fps: 30, frames: 12 } };
  // Anywhere inside a source frame targets that frame's middle, so a decoder
  // that joins late still presents the frame the compositor is drawing.
  for (const at of [3 / 30, 3.25 / 30, 3.99 / 30]) expect(sequenceStreamTime(asset, at)).toBeCloseTo(3.5 / 30, 6);
  expect(sequenceStreamTime(asset, 0)).toBeCloseTo(.5 / 30, 6);
  // Past the end holds the last frame; before the start holds the first.
  expect(sequenceStreamTime(asset, 20)).toBeCloseTo(11.5 / 30, 6);
  expect(sequenceStreamTime(asset, -2)).toBeCloseTo(.5 / 30, 6);
  expect(sequenceStreamTime({}, 1)).toBeUndefined();
  // Every target stays strictly inside its own frame, at any importable rate.
  const fast = { imageSequence: { fps: 240, frames: 8 } };
  for (let index = 0; index < 8; index++) {
    const at = sequenceStreamTime(fast, index / 240)!;
    expect(Math.floor(at * 240)).toBe(index);
  }
});


it('accepts sequences longer than 20,000 frames without dropping frames', () => {
  const frames = Array.from({ length: 20_001 }, (_, index) => ({ name: `frame_${20_000 - index}.png` }));
  const sorted = orderedSequence(frames);
  expect(sorted).toHaveLength(20_001);
  expect(sorted[0]?.name).toBe('frame_0.png');
  expect(sorted.at(-1)?.name).toBe('frame_20000.png');
});

/* Replays what the engine does to a decoder at a clip's in point: the element
   is told where to be once, then free-runs on its own clock while the
   compositor draws whole frames. Aiming at a frame's leading edge made any
   start-up lag present the previous frame, so the clip's first frame was drawn
   twice and every frame after it was one late. */
function playedFrames(asset: { imageSequence: { fps: number; frames: number } },
  { fps, from, rafPhase, startupLag }: { fps: number; from: number; rafPhase: number; startupLag: number }) {
  const shown: number[] = [];
  let elementTime = 0, elementClock = 0, started = false, lastDrawn = NaN;
  for (let tick = -4; tick < 40; tick++) {
    const now = from + rafPhase + tick / 120;
    const drawn = Math.floor(now * fps) / fps;
    if (now < from) continue;
    if (!started) { started = true; elementTime = sequenceStreamTime(asset, drawn - from)!; elementClock = now + startupLag; }
    if (drawn === lastDrawn) continue;
    lastDrawn = drawn;
    shown.push(Math.floor((elementTime + Math.max(0, now - elementClock)) * asset.imageSequence.fps + 1e-9));
  }
  return shown;
}

it('plays a clip from its in point without repeating its first frame', () => {
  const asset = { imageSequence: { fps: 30, frames: 40 } };
  for (const rafPhase of [0, .003, .006, .009, .012, .0155]) {
    for (const startupLag of [.002, .005, .008, .012, .016]) {
      const shown = playedFrames(asset, { fps: 30, from: 1, rafPhase, startupLag }).slice(0, 12);
      expect({ rafPhase, startupLag, shown }).toEqual({ rafPhase, startupLag, shown: shown.map((_, index) => index) });
    }
  }
});
