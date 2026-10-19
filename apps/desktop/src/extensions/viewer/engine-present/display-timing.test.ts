import { describe, expect, it } from 'vitest';

import { hintChanged, nextVsync, VsyncEstimator } from './display-timing';

/** rAF timestamps on a 120 Hz grid with ±0.5 ms jitter, every 7th skipped. */
function rafs(count: number, phaseMs: number, start = 1000): number[] {
  const period = 1000 / 120;
  const out: number[] = [];
  let seed = 7;
  for (let i = 0; i < count; i++) {
    if (i % 7 === 3) continue;
    seed = (seed * 1103515245 + 12345) % 2 ** 31;
    const jitter = (seed / 2 ** 31 - 0.5) * 1.0;
    out.push(Math.round((start + phaseMs + i * period + jitter) * 10) / 10);
  }
  return out;
}

describe('VsyncEstimator', () => {
  it('finds the phase of a jittered 120 Hz grid with the nominal period', () => {
    const estimator = new VsyncEstimator(120);
    let fit = null;
    for (const t of rafs(400, 2.0)) fit = estimator.sample(t) ?? fit;
    expect(fit).not.toBeNull();
    expect(fit!.periodMs).toBeCloseTo(1000 / 120, 9);
    const residual = ((fit!.vsyncMs - 1002.0) % fit!.periodMs + fit!.periodMs) % fit!.periodMs;
    expect(Math.min(residual, fit!.periodMs - residual)).toBeLessThan(0.3);
  });

  it('estimates the period across jittered refreshes without a nominal rate', () => {
    const estimator = new VsyncEstimator(null);
    for (const t of rafs(200, 0)) estimator.sample(t);
    expect(estimator.fit().periodMs).toBeGreaterThan(8.0);
    expect(estimator.fit().periodMs).toBeLessThan(8.7);
  });

  it('preserves fractional refresh periods despite 0.1 ms timestamp rounding and skipped refreshes', () => {
    for (const hz of [60, 120, 119.88, 144]) {
      const estimator = new VsyncEstimator();
      const period = 1000 / hz;
      for (let i = 0; i < 800; i++) {
        if (i % 13 === 5) continue;
        estimator.sample(Math.round((1000 + i * period) * 10) / 10);
      }
      expect(Math.abs(estimator.fit().periodMs - period)).toBeLessThan(0.0001);
    }
  });

  it('follows a half-period phase jump within a dozen refreshes', () => {
    const estimator = new VsyncEstimator(120);
    const before = rafs(300, 0, 1000);
    for (const t of before) estimator.sample(t);
    const last = before[before.length - 1]!;
    const after = rafs(40, 4.2, last + 50);
    let fit = estimator.current();
    for (const t of after) fit = estimator.sample(t) ?? fit;
    const target = last + 50 + 4.2;
    const residual = ((fit!.vsyncMs - target) % fit!.periodMs + fit!.periodMs) % fit!.periodMs;
    expect(Math.min(residual, fit!.periodMs - residual)).toBeLessThan(0.5);
  });

  it('keeps an inferred fractional period when the display link changes phase', () => {
    const estimator = new VsyncEstimator();
    const period = 1000 / 120;
    for (let i = 0; i < 320; i++) estimator.sample(Math.round((1000 + i * period + (i >= 280 ? 4.2 : 0)) * 10) / 10);
    expect(Math.abs(estimator.fit().periodMs - period)).toBeLessThan(0.001);
    const phase = ((estimator.fit().vsyncMs - 1004.2) % period + period) % period;
    expect(Math.min(phase, period - phase)).toBeLessThan(0.1);
  });

  it('places the next vsync on the grid', () => {
    const fit = { periodMs: 10, vsyncMs: 3, jumped: false, medianResidualMs: 0, samples: 100 };
    expect(nextVsync(fit, 3)).toBe(3);
    expect(nextVsync(fit, 3.1)).toBe(13);
    expect(nextVsync(fit, -20)).toBe(-17);
  });

  it('re-sends the vsync hint only for a new period or a phase move beyond 250 µs', () => {
    const base = { hostTime: 1_000_000, periodUs: 8333.333 };
    expect(hintChanged(null, base)).toBe(true);
    expect(hintChanged(base, { ...base, hostTime: base.hostTime + 8333.333 * 40 + 100 })).toBe(false);
    expect(hintChanged(base, { ...base, hostTime: base.hostTime + 400 })).toBe(true);
    expect(hintChanged(base, { ...base, hostTime: base.hostTime - 8333.333 + 50 })).toBe(false);
    expect(hintChanged(base, { hostTime: base.hostTime, periodUs: 16666.667 })).toBe(true);
  });
});
