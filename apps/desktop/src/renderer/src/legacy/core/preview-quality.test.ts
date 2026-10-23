import { describe, expect, it, vi } from 'vitest';
import { createPreviewQualityMonitor } from './preview-quality';

// The previous implementation is an independent compatibility oracle: it
// orders a fresh copy of the arrival window rather than maintaining an index.
function previousMonitor() {
  let previous: number | null = null, previouslyPlaying = false;
  const idleIntervals: number[] = [];
  let displayInterval = 1000 / 60;
  let start = Infinity, frames = 0, failed = 0, uncached = 0, slowWindows = 0;
  let target = 0;
  const clearWindow = () => { frames = 0; failed = 0; uncached = 0; };
  const reset = (now: number) => {
    start = now + 750;
    clearWindow(); slowWindows = 0;
  };
  return {
    reset,
    ready(now: number) { return now >= start; },
    budget(fps: number) { return 1000 / Math.min(fps, 1000 / displayInterval); },
    tick(now: number, playing: boolean, fps: number): boolean {
      const gap = previous === null ? 0 : now - previous;
      if (!playing && !previouslyPlaying && gap >= 4 && gap <= 100) {
        idleIntervals.push(gap);
        if (idleIntervals.length > 30) idleIntervals.shift();
        if (idleIntervals.length >= 5) {
          const sorted = [...idleIntervals].sort((a, b) => a - b);
          displayInterval = sorted[Math.floor(sorted.length / 4)]!;
        }
      }
      const changed = playing !== previouslyPlaying || fps !== target || gap > 1000 || gap < 0;
      previous = now; previouslyPlaying = playing; target = fps;
      if (changed) reset(now);
      if (!playing || now < start + 1000) return false;
      const expected = Math.min(fps, 1000 / displayInterval);
      const rate = frames * 1000 / (now - start);
      const overloaded = frames >= 3 && failed <= frames / 9
        && uncached >= frames / 2 && rate < expected * .8;
      slowWindows = overloaded ? slowWindows + 1 : 0;
      start = now; clearWindow();
      return slowWindows >= 2;
    },
    record(now: number, presented: boolean, cached = false) {
      if (now < start) return;
      if (!presented) { failed++; return; }
      frames++;
      if (!cached) uncached++;
    },
  };
}

function random(seed: number) {
  return () => {
    seed = Math.imul(seed, 1664525) + 1013904223 | 0;
    return (seed >>> 0) / 0x100000000;
  };
}

describe('preview display cadence', () => {
  it('learns after five valid gaps and selects the same lower quartile', () => {
    const monitor = createPreviewQualityMonitor();
    let now = 0;
    monitor.tick(now, false, 240);
    for (const gap of [8, 9, 10, 11]) monitor.tick(now += gap, false, 240);
    expect(monitor.budget(240)).toBe(1000 / 60);
    monitor.tick(now += 12, false, 240);
    expect(monitor.budget(240)).toBe(9);
  });

  it('expires samples by arrival order, including duplicates and changing displays', () => {
    const monitor = createPreviewQualityMonitor();
    let now = 0;
    monitor.tick(now, false, 240);
    for (let sample = 0; sample < 30; sample++) monitor.tick(now += 8, false, 240);
    expect(monitor.budget(240)).toBe(8);
    for (let sample = 0; sample < 22; sample++) monitor.tick(now += 16, false, 240);
    expect(monitor.budget(240)).toBe(8);
    monitor.tick(now += 16, false, 240);
    expect(monitor.budget(240)).toBe(16);
    for (let sample = 0; sample < 8; sample++) monitor.tick(now += 8, false, 240);
    expect(monitor.budget(240)).toBe(8);
  });

  it('accepts both gap boundaries and ignores tiny, backward, and suspended gaps', () => {
    const monitor = createPreviewQualityMonitor();
    let now = 0;
    monitor.tick(now, false, 240);
    for (const gap of [100, 4, 100, 100, 100]) monitor.tick(now += gap, false, 240);
    expect(monitor.budget(240)).toBe(100);
    for (const gap of [0, 3.999, -4, 100.001, 1001]) monitor.tick(now += gap, false, 240);
    expect(monitor.budget(240)).toBe(100);
  });

  it('keeps the learned display baseline through reset and playback, then resumes on the second paused tick', () => {
    const monitor = createPreviewQualityMonitor();
    let now = 0;
    monitor.tick(now, false, 240);
    for (let sample = 0; sample < 30; sample++) monitor.tick(now += 8, false, 240);
    monitor.reset(now);
    for (let sample = 0; sample < 40; sample++) monitor.tick(now += 100, true, 240);
    expect(monitor.budget(240)).toBe(8);
    monitor.tick(now += 100, false, 240);
    for (let sample = 0; sample < 22; sample++) monitor.tick(now += 100, false, 240);
    expect(monitor.budget(240)).toBe(8);
    monitor.tick(now += 100, false, 240);
    expect(monitor.budget(240)).toBe(100);
  });

  it('performs no full-array sorting during a long paused session', () => {
    const monitor = createPreviewQualityMonitor();
    const sort = vi.spyOn(Array.prototype, 'sort');
    try {
      for (let frame = 0; frame < 10_000; frame++) monitor.tick(frame * 1000 / 120, false, 30);
      expect(sort).not.toHaveBeenCalled();
      expect(monitor.budget(240)).toBeCloseTo(1000 / 120, 8);
    } finally { sort.mockRestore(); }
  });

  it('reports sustained presentation overload after two windows and resets that history', () => {
    const monitor = createPreviewQualityMonitor();
    monitor.tick(0, true, 60);
    monitor.tick(750, true, 60);
    for (const now of [800, 1000, 1200]) monitor.record(now, true);
    expect(monitor.tick(1750, true, 60)).toBe(false);
    for (const now of [1900, 2100, 2300]) monitor.record(now, true);
    expect(monitor.tick(2750, true, 60)).toBe(true);
    monitor.reset(2750);
    monitor.tick(3500, true, 60);
    for (const now of [3500, 3700, 3900]) monitor.record(now, true);
    expect(monitor.tick(4500, true, 60)).toBe(false);
  });

  it('matches the previous monitor across seeded idle, playback, presentation, and reset sequences', () => {
    const gaps = [0, 3.999, 4, 4.001, 8, 1000 / 120, 1000 / 60, 33, 99.999, 100, 100.001, 300, 1000, 1001, -25];
    const rates = [24, 30, 60, 120, 240];
    for (let seed = 1; seed <= 120; seed++) {
      const next = random(seed), before = previousMonitor(), after = createPreviewQualityMonitor();
      let now = 0, playing = false, fps = 60;
      for (let step = 0; step < 600; step++) {
        now += gaps[Math.floor(next() * gaps.length)]!;
        if (next() < .04) playing = !playing;
        if (next() < .03) fps = rates[Math.floor(next() * rates.length)]!;
        if (next() < .02) { before.reset(now); after.reset(now); }
        const presented = next() >= .1, cached = next() < .2;
        before.record(now, presented, cached); after.record(now, presented, cached);
        expect(after.tick(now, playing, fps), `seed ${seed}, step ${step}`).toBe(before.tick(now, playing, fps));
        expect(after.ready(now)).toBe(before.ready(now));
        expect(after.ready(now + 750)).toBe(before.ready(now + 750));
        expect(after.budget(fps)).toBe(before.budget(fps));
        expect(after.budget(240)).toBe(before.budget(240));
      }
    }
  });

  it('preserves numeric edge behavior without admitting anomalous timestamps into the idle window', () => {
    const before = previousMonitor(), after = createPreviewQualityMonitor();
    const times = [0, -0, 4, 8, 12, 16, 20, NaN, 40, 48, Infinity, 64, -Infinity, 80, 88, 96, 104, 112];
    const rates = [0, -0, NaN, Infinity, -Infinity, 1, 240];
    for (let index = 0; index < times.length; index++) {
      const now = times[index]!, playing = index % 7 === 0, fps = rates[index % rates.length]!;
      if (index % 5 === 0) { before.reset(now); after.reset(now); }
      before.record(now, index % 3 !== 0, index % 4 === 0);
      after.record(now, index % 3 !== 0, index % 4 === 0);
      expect(after.tick(now, playing, fps)).toBe(before.tick(now, playing, fps));
      for (const value of rates) {
        expect(after.budget(value)).toBe(before.budget(value));
        expect(after.ready(value)).toBe(before.ready(value));
      }
    }
  });
});
