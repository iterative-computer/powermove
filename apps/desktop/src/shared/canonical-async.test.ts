import { describe, expect, it } from 'vitest';
import { canonicalStringify, jsonClone } from './canonical-json';
import { canonicalChunks, jsonCloneAsync } from './canonical-async';

async function text(value: any, chunkBytes?: number): Promise<string> {
  const decoder = new TextDecoder();
  let result = '';
  for await (const chunk of canonicalChunks(value, { chunkBytes, yieldTask: async () => {} })) {
    if (chunkBytes) expect(chunk.byteLength).toBeLessThanOrEqual(Math.min(chunkBytes, 256 * 1024));
    result += decoder.decode(chunk, { stream: true });
  }
  return result + decoder.decode();
}

function samples(): any[] {
  const odd = { toJSON: (key: string) => ({ key, n: new Number(3), s: new String('boxed'), b: new Boolean(false) }) };
  const sparse: any[] = [1, , 3]; // eslint-disable-line no-sparse-arrays
  const shared = { k: 1 };
  return [
    { b: [1, -0, 1e21, 1e-7, NaN, Infinity, -Infinity], a: 'é😀', lone: 'a\ud800b', u: undefined, f: () => 1, s: Symbol('s'), d: new Date(0), nested: { odd, sparse, list: [undefined, () => 2, Symbol('y'), null] } },
    JSON.parse('{"__proto__":{"x":1},"y":2}'), [odd, odd], 'top', 12, -0, null, true, new String('s'), new Uint8Array([1, 2]), new Map([[1, 2]]),
    { '10': 3, '2': 4, '': 5, A: 6, '\uffff': 1, '😀': 2 }, { a: shared, b: shared },
    '\u0000\u001f\b\f\n\r\t"\\/\u007f\u2028\udfff\ud800😀',
  ];
}

describe('cooperative JSON', () => {
  it('matches the existing clone, canonical form and JSON round trip', async () => {
    for (const value of samples()) {
      const copy = await jsonCloneAsync(value);
      expect(copy).toEqual(jsonClone(value));
      expect(JSON.stringify(copy)).toBe(JSON.stringify(JSON.parse(JSON.stringify(value))));
      expect(await text(value, 19)).toBe(canonicalStringify(value));
    }
    const shared = { k: 1 };
    const copy = await jsonCloneAsync({ a: shared, b: shared });
    expect(copy.a).not.toBe(copy.b);
    expect(copy.a).not.toBe(shared);
    expect(Object.is(await jsonCloneAsync(-0), 0)).toBe(true);
    const proto = await jsonCloneAsync(JSON.parse('{"__proto__":{"x":1}}'));
    expect(Object.getPrototypeOf(proto)).toBe(Object.prototype);
    expect(Object.hasOwn(proto, '__proto__')).toBe(true);
  });

  it('applies hooks once at their location, including self and returned hooks', async () => {
    for (const operation of [jsonCloneAsync, text]) {
      const calls: string[] = [];
      const self: any = { value: 1, nested: { toJSON(key: string) { calls.push(`nested:${key}`); return 'saved'; } } };
      Object.defineProperty(self, 'toJSON', { value(key: string) { calls.push(`self:${key}`); return this; } });
      await operation({ channel: self });
      expect(calls).toEqual(['self:channel', 'nested:nested']);
      let returnedCalls = 0;
      const returned = { value: 3, toJSON() { returnedCalls++; return { value: 4 }; } };
      const outer = { toJSON() { return returned; } };
      expect(await text({ channel: outer })).toBe('{"channel":{"value":3}}');
      expect(await operation({ channel: outer })).toBeDefined();
      expect(returnedCalls).toBe(0);
      const keys: string[] = [];
      await operation([0, 1, { toJSON(key: string) { keys.push(key); return undefined; } }]);
      expect(keys).toEqual(['2']);
    }
  });

  it('captures object keys before child hooks change later membership', async () => {
    const make = () => {
      const root: any = Object.create({ z: 'inherited' });
      root.a = { toJSON() { delete root.z; return 1; } };
      root.z = 'own';
      return root;
    };
    expect(await jsonCloneAsync(make())).toEqual(JSON.parse(JSON.stringify(make())));
    expect(await text(make())).toBe(canonicalStringify(make()));
  });

  it('handles absent roots, cycles, BigInts and repeated references', async () => {
    for (const value of [undefined, () => 1, Symbol('x'), { toJSON: () => undefined }]) {
      expect(await jsonCloneAsync(value)).toBeUndefined();
      await expect(text(value)).rejects.toThrow(/no JSON representation/);
    }
    const cycle: any = { a: 1 }; cycle.self = cycle;
    const selfHook: any = { value: 1 }; selfHook.self = selfHook;
    Object.defineProperty(selfHook, 'toJSON', { value() { return this; } });
    for (const value of [cycle, selfHook, 1n, { n: 1n }]) {
      await expect(jsonCloneAsync(value)).rejects.toThrow(TypeError);
      await expect(text(value)).rejects.toThrow(TypeError);
    }
  });

  it('quotes giant scalars in bounded pieces without splitting surrogate pairs', async () => {
    const value = '\ud800😀é\n\\"'.repeat(50_000);
    expect(await text(value, 257)).toBe(canonicalStringify(value));
    expect(await text('x'.repeat(400_000), 1_000_000)).toBe(canonicalStringify('x'.repeat(400_000)));
    await expect(text('a', 0)).rejects.toThrow(RangeError);
  });

  it('yields during large arrays, wide key sorting and giant strings', async () => {
    let reads = 0, lastReads = 0, maxReads = 0, yields = 0;
    const array = new Proxy(Array.from({ length: 20_000 }, (_, i) => i), {
      get(target, key, receiver) { if (typeof key === 'string' && /^\d+$/.test(key)) reads++; return Reflect.get(target, key, receiver); }
    });
    const yieldTask = async () => { maxReads = Math.max(maxReads, reads - lastReads); lastReads = reads; yields++; };
    expect((await jsonCloneAsync(array, { yieldTask })).length).toBe(20_000);
    expect(yields).toBeGreaterThan(10);
    expect(maxReads).toBeLessThanOrEqual(256);
    yields = 0;
    const wide = Object.fromEntries(Array.from({ length: 4000 }, (_, i) => [`key${4000 - i}`, i]));
    let bytes = 0;
    for await (const chunk of canonicalChunks({ wide, string: '😀'.repeat(100_000) }, { yieldTask })) bytes += chunk.length;
    expect(yields).toBeGreaterThan(10);
    expect(bytes).toBe(new TextEncoder().encode(canonicalStringify({ wide, string: '😀'.repeat(100_000) })!).length);
  });

  it('handles deep trees with an ancestor cursor instead of the JS call stack', async () => {
    const root: any = {};
    let cursor = root;
    for (let i = 0; i < 12_000; i++) cursor = cursor.next = {};
    const copy = await jsonCloneAsync(root, { yieldTask: async () => {} });
    let depth = 0, at = copy;
    while (at.next) { depth++; at = at.next; }
    expect(depth).toBe(12_000);
    let bytes = 0;
    for await (const chunk of canonicalChunks(root, { yieldTask: async () => {} })) bytes += chunk.length;
    expect(bytes).toBe(12_000 * 9 + 2);
  });

  it('allows an injected task scheduler to service queued messages', async () => {
    let serviced = false;
    const queued = new Promise<void>(resolve => setTimeout(() => { serviced = true; resolve(); }, 0));
    await jsonCloneAsync(Array.from({ length: 5000 }, (_, i) => i), { yieldTask: () => queued });
    expect(serviced).toBe(true);
  });
});
