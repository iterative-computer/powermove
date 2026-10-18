import { describe, expect, it, vi } from 'vitest';
import { appUiFontCacheSeeds, effectiveFontSize, FontSeedTracker, seedsJson } from './engine-fonts-seeds';

describe('font cache seeds', () => {
  it('are the UI text entries at each scale factor, as pm-text computes them', () => {
    // pm-text blink::measure seed_tests pins the same values.
    expect(appUiFontCacheSeeds([2]).map(s => [s.size, s.specified])).toEqual([[24, 12], [26, 13]]);
    expect(appUiFontCacheSeeds([2]).every(s => s.family === 'system-ui' && s.weight === 400 && !s.italic)).toBe(true);
    expect(appUiFontCacheSeeds([1]).every(s => s.size === s.specified)).toBe(true);
    const both = appUiFontCacheSeeds([2, Number.NaN, 1, 2]);
    expect(both).toHaveLength(4);
    expect(both[0]!.size).toBe(24);
    expect(appUiFontCacheSeeds([1.1])[0]!.size).toBe(Math.fround(13.2));
    expect(appUiFontCacheSeeds([])).toEqual([]);
    expect(seedsJson(appUiFontCacheSeeds([2]))).toBe('{"seeds":[{"family":"system-ui","weight":400,"italic":false,"size":24,"specified":12},{"family":"system-ui","weight":400,"italic":false,"size":26,"specified":13}]}');
  });

  it('floor the effective size as Blink does, in float', () => {
    // pm-text compat::canvas pins the f32 steps: 1.06 measures at 1.05.
    expect(effectiveFontSize(1.06)).toBe(Math.fround(1.05));
    expect(effectiveFontSize(11.8675)).toBe(Math.fround(11.86));
  });

  it('follow every window\'s scale factors and announce changes', () => {
    const tracker = new FontSeedTracker();
    const changed = vi.fn();
    tracker.onChange(changed);
    tracker.observe(1, 2);
    tracker.observe(1, 2);
    expect(changed).toHaveBeenCalledTimes(1);
    expect(tracker.seedsFor(1).map(s => s.size)).toEqual([24, 26]);
    // Moving to a 1x display keeps the 2x entries.
    tracker.observe(1, 1);
    expect(tracker.seedsFor(1).map(s => s.size)).toEqual([24, 26, 12, 13]);
    tracker.observe(2, 1);
    expect(tracker.seedsFor(2).map(s => s.size)).toEqual([12, 13]);
    expect(tracker.allSeeds().map(s => s.size)).toEqual([24, 26, 12, 13]);
    expect(tracker.seedsFor(9)).toEqual([]);
    tracker.forget(1);
    expect(changed).toHaveBeenLastCalledWith(null);
    expect(tracker.allSeeds().map(s => s.size)).toEqual([12, 13]);
    tracker.observe(3, Number.NaN);
    expect(tracker.seedsFor(3)).toEqual([]);
  });
});
