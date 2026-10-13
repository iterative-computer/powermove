import { describe, expect, it } from 'vitest';
import { randomPair } from '../../../../../../engine/crates/pm-model/tests/vectors/generate-patch-binary-vectors';
import { applyOps, diffOps } from '../../../shared/patch-ids';
import { canonicalStringify } from '../../../shared/canonical-json';
import { diffJsonOps, sameJson } from './json-diff';

const json = (v: unknown) => (v === undefined ? undefined : JSON.parse(JSON.stringify(v)));

describe('diffJsonOps', () => {
  it('emits exactly what diffOps emits on plain JSON', () => {
    for (let seed = 1; seed <= 2000; seed++) {
      const [before, after] = randomPair(seed);
      expect(diffJsonOps(before, after), `seed ${seed}`).toEqual(diffOps(before, after));
    }
  });

  it('turns the shadow into the live document as JSON.stringify sees it', () => {
    const before = { a: 1, keys: [{ t: 0, v: 1, eo: [0.3, 0], ei: [0.7, 1] }], n: 5, z: 1 };
    const key: any = { t: 0, v: 1, outEase: { speed: 0, influence: 33 } };
    Object.defineProperty(key, 'eo', { enumerable: false, configurable: true, get: () => [0.3, 0] });
    const live: any = { a: 1, keys: [key], n: NaN, z: -0, gone: undefined, fn: () => 1, date: { toJSON: () => 'D' } };
    const ops = diffJsonOps(before, live);
    const result = applyOps(json(before), ops);
    expect(canonicalStringify(result)).toBe(canonicalStringify(live));
    // A second diff against the result is empty: nothing oscillates.
    expect(diffJsonOps(result, live)).toEqual([]);
  });

  it('handles a member that has no JSON form at the root path', () => {
    expect(diffJsonOps({ a: 1 }, undefined, ['x'])).toEqual([{ path: ['x'], exists: true }]);
    expect(diffJsonOps(undefined, undefined, ['x'])).toEqual([]);
  });

  it('sameJson agrees with an empty diff', () => {
    for (let seed = 1; seed <= 2000; seed++) {
      const [before, after] = randomPair(seed);
      expect(sameJson(before, after), `seed ${seed}`).toBe(diffJsonOps(before, after).length === 0);
      expect(sameJson(before, json(before)), `seed ${seed}`).toBe(true);
    }
    const key: any = { t: 0, v: 1 };
    Object.defineProperty(key, 'eo', { enumerable: false, get: () => [0.3, 0] });
    const live: any = { a: [1, undefined, () => 1], n: NaN, z: -0, gone: undefined, date: { toJSON: () => 'D' }, key };
    const shadow = json(live);
    expect(sameJson(shadow, live)).toBe(true);
    expect(diffJsonOps(shadow, live)).toEqual([]);
    for (const change of [{ n: 1 }, { z: 1 }, { gone: 0 }, { date: 'E' }, { extra: null }]) {
      const other = { ...live, ...change };
      expect(sameJson(shadow, other), JSON.stringify(change)).toBe(diffJsonOps(shadow, other).length === 0);
    }
    // Members inherited from Object.prototype are not JSON members.
    expect(sameJson({ a: 1 }, { a: 1, toString: 1 })).toBe(false);
    expect(sameJson({ a: 1, toString: 1 }, { a: 1 })).toBe(false);
    expect(sameJson({ constructor: 1 }, { valueOf: 1 })).toBe(false);
  });
});
