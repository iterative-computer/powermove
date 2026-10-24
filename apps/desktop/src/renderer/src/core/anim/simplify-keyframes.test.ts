import { describe, expect, it, vi } from 'vitest';
import { simplifyKeyframes, type SimplifyOptions } from './simplify-keyframes';
import { linearKeyframeEase } from './temporal-ease';
import { makePM } from '../../legacy/__tests__/make-pm';

const PM = makePM('core/easing', 'core/anim');
const keys = (values: number[], times = values.map((_, i) => i)) => values.map((v, i) =>
  ({ i: `k${i}`, t: times[i]!, v, ...linearKeyframeEase() }));
const options = (overrides: Partial<SimplifyOptions> = {}): SimplifyOptions => ({
  evaluate: PM.evalKfs, fps: 30, from: 0, duration: 10, compositionDuration: 10,
  ...overrides,
});
function compare(before: any[], after: any[]) {
  for (let i = -120; i <= 1200; i++) expect(PM.evalKfs(after, i / 120)).toBeCloseTo(PM.evalKfs(before, i / 120), 9);
}

describe('verified keyframe simplification using the real evaluator', () => {
  it('collapses long equal runs while keeping both ends of each changing segment', () => {
    const source = keys([2, 2, 2, 2, 9, 9, 9, 4, 4, 4]);
    const result = simplifyKeyframes(source, options());
    expect(result.map(key => key.i)).toEqual(['k0', 'k3', 'k4', 'k6', 'k7', 'k9']);
    compare(source, result);
  });
  it('keeps hold transitions at their original times', () => {
    const source = keys([2, 2, 2, 9, 9, 9]);
    source.forEach(key => key.outInterp = 'hold');
    const result = simplifyKeyframes(source, options());
    expect(result.map(key => key.t)).toEqual([0, 2, 3, 5]);
    compare(source, result);
  });
  it('simplifies flat zero-speed Bezier segments', () => {
    const source = keys([3, 3, 3, 3]);
    source.forEach(key => { key.inInterp = key.outInterp = 'bezier'; });
    expect(simplifyKeyframes(source, options())).toHaveLength(2);
  });
  it('retains equal-valued Beziers with non-zero speed and their overshoot', () => {
    const source = keys([3, 3, 3, 3]);
    source.forEach(key => { key.inInterp = key.outInterp = 'bezier'; key.outEase.speed = 12; key.inEase.speed = -12; });
    expect(PM.evalKfs(source, .5)).not.toBe(3);
    expect(simplifyKeyframes(source, options())).toBe(source);
  });
  it('retains Beziers whose flat controls suffer floating point cancellation', () => {
    const source = keys([.3, .3, .3]);
    source.forEach(key => { key.inInterp = key.outInterp = 'bezier'; });
    expect(simplifyKeyframes(source, options())).toBe(source);
  });
  it.each(['autoBezier', 'continuous'] as const)('keeps %s tracks, including tangent-sensitive neighbours', flag => {
    const source = keys([0, 8, 8, 8, 16]); source[1]![flag] = true;
    const saved = JSON.stringify(source);
    expect(simplifyKeyframes(source, options())).toBe(source);
    expect(JSON.stringify(source)).toBe(saved);
  });
  it('skips expressions even when sampled values would agree', () => {
    const source = keys([8, 8, 8]);
    expect(simplifyKeyframes(source, options({ expression: 'loopOut()' }))).toBe(source);
  });
  it('trims beyond both visible edges, retaining the nearest outside keys and shutter margin', () => {
    const source = keys([10, 20, 30, 40, 50, 60, 70], [-5, -1, 0, .5, 1, 2, 5]);
    const result = simplifyKeyframes(source, options({ trimOutside: true, duration: 1 }));
    expect(result.map(key => key.t)).toEqual([-1, 0, .5, 1, 2]);
    for (let i = -2; i <= 122; i++) expect(PM.evalKfs(result, i / 120)).toBe(PM.evalKfs(source, i / 120));
  });
  it('does not trim time-remapped/nested tracks when trimming is disabled', () => {
    const source = keys([10, 20, 30, 40, 50], [-5, -1, .5, 2, 5]);
    expect(simplifyKeyframes(source, options({ trimOutside: false, duration: 1 }))).toBe(source);
  });
  it('handles separated dimensions independently', () => {
    const x = keys([8, 8, 8, 8]), y = keys([0, 4, 8, 16]);
    expect(simplifyKeyframes(x, options())).toHaveLength(2);
    expect(simplifyKeyframes(y, options())).toBe(y);
  });
  it('is pure for legacy easing and idempotent', () => {
    const source = [0, 1, 2, 3].map(t => ({ t, v: 12, i: `k${t}`, eo: [.33, 0], ei: [.67, 1] }));
    const saved = JSON.stringify(source);
    const result = simplifyKeyframes(source, options());
    expect(result).toHaveLength(2);
    expect(JSON.stringify(source)).toBe(saved);
    expect(simplifyKeyframes(result, options())).toBe(result);
  });
  it('verifies all frame and motion-blur samples using the supplied real evaluator', () => {
    const evaluate = vi.fn((track: any[], time: number) => PM.evalKfs(track, time)), source = keys([1, 1, 1]);
    simplifyKeyframes(source, options({ evaluate, duration: 2, compositionDuration: 2 }));
    for (let frame = 0; frame <= 60; frame++) {
      const times = evaluate.mock.calls.map(call => call[1]);
      expect(times).toContain(frame / 30);
      for (let sample = 0; sample < 10; sample++) expect(times).toContain(frame / 30 + ((sample + .5) / 10 - .5) * (.5 / 30));
    }
  });
  it('rejects a candidate if the evaluator detects even a sub-frame difference', () => {
    const source = keys([1, 1, 1]);
    const result = simplifyKeyframes(source, options({ evaluate: (track, time) => PM.evalKfs(track, time)
      + (track.length === 2 && time > .003 && time < .005 ? .01 : 0) }));
    expect(result).toBe(source);
  });
  it('rejects differences smaller than a rounding tolerance', () => {
    const source = keys([1, 1, 1]);
    expect(simplifyKeyframes(source, options({ evaluate: (track, time) => PM.evalKfs(track, time)
      + (track.length === 2 ? 1e-12 : 0) }))).toBe(source);
  });
  it('keeps legacy continuous tracks after ease migration', () => {
    const source = [0, 1, 2].map(t => ({ t, v: 1, bezierMode: 'continuous', eo: [.33, 0], ei: [.67, 1] }));
    expect(simplifyKeyframes(source, options())).toBe(source);
  });
  it.each([
    [{ t: 0, v: [0, 1] }, { t: 1, v: [0, 1] }, { t: 2, v: [0, 1] }],
    [{ t: 0, v: 1, spatialTangent: [1, 1] }, { t: 1, v: 1 }, { t: 2, v: 1 }],
    [{ t: 0, v: 1, spring: { stiffness: 20 } }, { t: 1, v: 1 }, { t: 2, v: 1 }],
    [{ t: 0, v: 1 }, { t: 0, v: 1 }, { t: 2, v: 1 }],
  ].map(source => ({ source })))('keeps structured, spatial, spring and malformed tracks', ({ source }) => {
    expect(simplifyKeyframes(source as Record<string, any>[], options())).toBe(source);
  });
});
