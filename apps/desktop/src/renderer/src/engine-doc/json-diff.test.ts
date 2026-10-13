import { describe, expect, it } from 'vitest';
import { randomPair } from '../../../../../../engine/crates/pm-model/tests/vectors/generate-patch-binary-vectors';
import { applyOps, diffOps } from '../../../shared/patch-ids';
import { canonicalStringify } from '../../../shared/canonical-json';
import { diffJsonOps } from './json-diff';

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
});
