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

describe('sameJson fast path', () => {
  it('agrees with the full comparison on document shapes and on the exceptions', () => {
    const accessor: any = { t: 0, v: 1 };
    Object.defineProperty(accessor, 'eo', { enumerable: false, get: () => [0.3, 0] });
    const absent = { toJSON: () => undefined };
    const proto = Object.create({ inherited: 1 });
    proto.own = 1;
    const cases: Array<[any, any, boolean]> = [
      [{ a: 1, b: [1, 2] }, { b: [1, 2], a: 1 }, true],
      [{ a: 1 }, { a: 1, gone: absent }, true],
      [{ a: 1, gone: null }, { a: 1, gone: absent }, false],
      [{ a: 1 }, { a: 1, f: () => 1, s: Symbol('s'), u: undefined }, true],
      [{ a: [null, null] }, { a: [undefined, () => 1] }, true],
      [{ n: null, z: 0 }, { n: Infinity, z: -0 }, true],
      [{ n: 0 }, { n: NaN }, false],
      [{ k: accessor && { t: 0, v: 1 } }, { k: accessor }, true],
      [{ own: 1 }, proto, true],
      [{ d: '1970-01-01T00:00:00.000Z' }, { d: new Date(0) }, true],
      [{ w: 3 }, { w: new Number(3) }, true],
      [{ a: 1 }, { a: 1, toJSON: () => ({ a: 2 }) }, false],
      [{ a: 2 }, { a: 1, toJSON: () => ({ a: 2 }) }, true],
      [{ constructor: 1 }, { valueOf: 1 }, false],
      [{ a: 1, b: 2 }, { a: 1 }, false],
      [{ a: 1 }, { a: 1, b: 2 }, false],
      [[1], { 0: 1, length: 1 }, false],
      [{ x: 2 }, { x: 1, toJSON: () => ({ x: 2 }) }, true],
      [[1, 2], { toJSON: () => [1, 2] }, true],
      [{ a: [1, 3] }, { a: Object.assign([1, 2], { toJSON: () => [1, 3] }) }, true],
      ['T', { a: 1, toJSON: () => 'T' }, true],
    ];
    for (const [before, after, expected] of cases) {
      expect(sameJson(before, after), JSON.stringify(before)).toBe(expected);
      expect(sameJson(before, after)).toBe(diffJsonOps(before, after).length === 0);
    }
  });
});
