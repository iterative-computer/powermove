import { describe, expect, it } from 'vitest';

import { DEFAULT_FRAME_MS, MAX_SEQUENCE_FRAMES, animationTiming, frameDelaysMs } from './animated-image';
import { validSequenceFps } from './image-sequence';

const micros = (...ms: number[]) => ms.map(value => value * 1000);
const seconds = (timing: { fps: number; frames: number }) => timing.frames / timing.fps;

describe('animated image timing', () => {
  it('reproduces a single shared delay at its own frame rate', () => {
    const timing = animationTiming(micros(...Array(10).fill(100)));
    expect(timing).toEqual({ fps: 10, repeats: Array(10).fill(1), frames: 10 });
    expect(seconds(timing)).toBeCloseTo(1, 6);
  });

  it('holds each frame for its own delay when delays share a divisor', () => {
    const timing = animationTiming(micros(80, 80, 90));
    expect(timing).toEqual({ fps: 100, repeats: [8, 8, 9], frames: 25 });
    expect(seconds(timing)).toBeCloseTo(0.25, 6);
  });

  it('renders absent and impossibly short delays the way browsers do', () => {
    expect(frameDelaysMs([0, null, undefined, 4000, 20_000])).toEqual([
      DEFAULT_FRAME_MS, DEFAULT_FRAME_MS, DEFAULT_FRAME_MS, DEFAULT_FRAME_MS, 20,
    ]);
  });

  it('oversamples coprime delays instead of demanding an unplayable frame rate', () => {
    const delays = Array.from({ length: 12 }, (_, index) => (index % 2 ? 84 : 83));
    const timing = animationTiming(micros(...delays));
    // A 1 ms divisor would ask for 1000 fps; the proxy encoder tops out at 240.
    expect(validSequenceFps(timing.fps)).toBe(true);
    expect(timing.repeats).toHaveLength(delays.length);
    expect(timing.repeats.every(count => count >= 1)).toBe(true);
    expect(seconds(timing)).toBeCloseTo(delays.reduce((a, b) => a + b, 0) / 1000, 1);
  });

  it('keeps frame starts from drifting as an oversampled animation runs on', () => {
    const delays = Array.from({ length: 60 }, () => 33);
    const timing = animationTiming(micros(...delays), 240);
    let elapsed = 0, placed = 0, worst = 0;
    delays.forEach((ms, index) => {
      placed += timing.repeats[index]!;
      elapsed += ms / 1000;
      worst = Math.max(worst, Math.abs(placed / timing.fps - elapsed));
    });
    expect(worst).toBeLessThanOrEqual(1 / timing.fps);
  });

  it('stays inside the encoder frame budget for a long fine-grained animation', () => {
    const timing = animationTiming(micros(...Array(400).fill(17)));
    expect(timing.frames).toBeLessThanOrEqual(MAX_SEQUENCE_FRAMES);
    expect(validSequenceFps(timing.fps)).toBe(true);
  });

  it('refuses an animation with no frames at all', () => {
    expect(() => animationTiming([])).toThrow('no frames');
  });
});
