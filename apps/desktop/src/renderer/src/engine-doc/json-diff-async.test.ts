import { describe, expect, it } from 'vitest';
import { randomPair } from '../../../../../../engine/crates/pm-model/tests/vectors/generate-patch-binary-vectors';
import { sameJson } from './json-diff';
import { sameJsonAsync } from './json-diff-async';

const options = { yieldTask: async () => {} };
const json = (value: any) => JSON.parse(JSON.stringify(value));

describe('sameJsonAsync', () => {
  it('matches the synchronous alias check on generated documents', async () => {
    for (let seed = 1; seed <= 500; seed++) {
      const [before, after] = randomPair(seed);
      expect(await sameJsonAsync(before, after, options), `seed ${seed}`).toBe(sameJson(before, after));
      expect(await sameJsonAsync(before, json(before), options)).toBe(true);
    }
  });

  it('observes JSON omissions, custom prototypes, wrappers and non-enumerable views', async () => {
    const accessor: any = { t: 0, v: 1 };
    Object.defineProperty(accessor, 'eo', { enumerable: false, get: () => [0.3, 0] });
    const proto = Object.assign(Object.create({ inherited: 1 }), { own: 1 });
    const absent = { toJSON: () => undefined };
    const cases: Array<[any, any]> = [
      [{ a: 1, b: [1, 2] }, { b: [1, 2], a: 1 }], [{ a: 1 }, { a: 1, gone: absent }],
      [{ a: 1, gone: null }, { a: 1, gone: absent }], [{ a: 1 }, { a: 1, f: () => 1, s: Symbol('s'), u: undefined }],
      [{ a: [null, null] }, { a: [undefined, () => 1] }], [{ n: null, z: 0 }, { n: Infinity, z: -0 }],
      [{ n: 0 }, { n: NaN }], [{ k: { t: 0, v: 1 } }, { k: accessor }], [{ own: 1 }, proto],
      [{ d: '1970-01-01T00:00:00.000Z' }, { d: new Date(0) }], [{ w: 3 }, { w: new Number(3) }],
      [{ a: 1 }, { a: 1, toJSON: () => ({ a: 2 }) }], [{ a: 2 }, { a: 1, toJSON: () => ({ a: 2 }) }],
      [{ constructor: 1 }, { valueOf: 1 }], [{ a: 1, b: 2 }, { a: 1 }], [{ a: 1 }, { a: 1, b: 2 }],
      [[1], { 0: 1, length: 1 }], [{ x: 2 }, { x: 1, toJSON: () => ({ x: 2 }) }],
      [[1, 2], { toJSON: () => [1, 2] }], [{ a: [1, 3] }, { a: Object.assign([1, 2], { toJSON: () => [1, 3] }) }],
      ['T', { a: 1, toJSON: () => 'T' }], [undefined, { toJSON: () => undefined }],
      [JSON.parse('{"__proto__":1}'), JSON.parse('{"__proto__":1}')],
    ];
    for (const [before, after] of cases) expect(await sameJsonAsync(before, after, options)).toBe(sameJson(before, after));
  });

  it('calls self-returning and returned hooks exactly once with their member key', async () => {
    const calls: string[] = [];
    const self: any = { value: 1, nested: { toJSON(key: string) { calls.push(`nested:${key}`); return 'saved'; } } };
    Object.defineProperty(self, 'toJSON', { value(key: string) { calls.push(`self:${key}`); return this; } });
    const live = { channel: self };
    const shadow = json(live);
    calls.length = 0;
    expect(await sameJsonAsync(shadow, live, options)).toBe(true);
    expect(calls).toEqual(['self:channel', 'nested:nested']);
    self.value = 2;
    expect(await sameJsonAsync(shadow, live, options)).toBe(false);
    let returnedHookCalls = 0;
    const returned = { value: 3, toJSON() { returnedHookCalls++; return { value: 4 }; } };
    const outer = { toJSON() { return returned; } };
    expect(await sameJsonAsync(json({ channel: outer }), { channel: outer }, options)).toBe(true);
    expect(returnedHookCalls).toBe(0);
    const keys: string[] = [];
    const array = [0, 1, { toJSON(key: string) { keys.push(key); return undefined; } }];
    expect(await sameJsonAsync([0, 1, null], array, options)).toBe(true);
    expect(keys).toEqual(['2']);
  });

  it('validates whole hook subtrees for cycles and retains strict alias shortcuts', async () => {
    const cycle: any = { value: 1 }; cycle.self = cycle;
    Object.defineProperty(cycle, 'toJSON', { value() { return this; } });
    await expect(sameJsonAsync({ channel: { value: 2, self: {} } }, { channel: cycle }, options)).rejects.toThrow(/circular/i);
    expect(await sameJsonAsync(cycle, cycle, options)).toBe(true);
    expect(await sameJsonAsync({ channel: cycle }, { channel: cycle }, options)).toBe(true);
    const large = { toJSON: () => '😀'.repeat(100_000) };
    expect(await sameJsonAsync('😀'.repeat(100_000), large, options)).toBe(true);
  });

  it('preserves canonical visitation order inside the hook comparison fallback', async () => {
    const calls: string[] = [];
    const live = { toJSON: () => ({
      z: { toJSON(key: string) { calls.push(key); return key; } },
      a: { toJSON(key: string) { calls.push(key); return key; } },
    }) };
    expect(await sameJsonAsync({ a: 'a', z: 'z' }, live, options)).toBe(true);
    expect(calls).toEqual(['a', 'z']);
  });

  it('compares giant scalar strings cooperatively without whole-string serialization', async () => {
    let yields = 0;
    const value = '😀'.repeat(300_000);
    const other = ('x' + value).slice(1);
    expect(await sameJsonAsync(value, other, { yieldTask: async () => { yields++; } })).toBe(true);
    expect(yields).toBeGreaterThan(0);
    expect(await sameJsonAsync(value, value.slice(0, -1) + 'x', options)).toBe(false);
  });

  it('yields throughout a large comparison with a bounded array cursor', async () => {
    const shadow = Array.from({ length: 30_000 }, (_, i) => ({ value: i }));
    let reads = 0, last = 0, largest = 0, yields = 0;
    const live = new Proxy(json(shadow), { get(target, key, receiver) {
      if (typeof key === 'string' && /^\d+$/.test(key)) reads++;
      return Reflect.get(target, key, receiver);
    } });
    expect(await sameJsonAsync(shadow, live, { yieldTask: async () => {
      largest = Math.max(largest, reads - last); last = reads; yields++;
    } })).toBe(true);
    expect(yields).toBeGreaterThan(10);
    expect(largest).toBeLessThanOrEqual(256);
  });
});
