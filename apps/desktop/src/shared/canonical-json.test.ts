// Runs under `bun test` and vitest. The engine side (engine/crates/pm-model,
// tests/canonical.rs) checks itself against vectors generated from this module.
import { readFileSync, readdirSync } from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';
import { COMP_FIELDS, rotateProject } from '../renderer/src/legacy/core/compositions';
import { CANONICAL_COMP_FIELDS, canonicalHash, canonicalProject, canonicalProjectHash, canonicalStringify, realizeProject } from './canonical-json';
import { sha256Hex } from './sha256';

const engineTests = path.resolve(__dirname, '../../../../engine/crates/pm-model/tests');

describe('canonicalStringify', () => {
  it('sorts keys by UTF-16 code units and otherwise writes JSON.stringify output', () => {
    expect(canonicalStringify({ b: 1, a: 2, 10: 3, 2: 4, '': 5, A: 6 })).toBe('{"":5,"10":3,"2":4,"A":6,"a":2,"b":1}');
    // U+FFFF sorts after the surrogate pair of U+1F600 in UTF-16 order.
    expect(canonicalStringify({ '\uffff': 1, '\ud83d\ude00': 2 })).toBe('{"\ud83d\ude00":2,"\uffff":1}');
    expect(canonicalStringify([0, -0, 1e21, 1e-7, 0.1 + 0.2, NaN, Infinity, -Infinity])).toBe('[0,0,1e+21,1e-7,0.30000000000000004,null,null,null]');
    // A low surrogate before a high one: two lone surrogates.
    expect(canonicalStringify('\u0000\u001f\b\f\n\r\t"\\/\u007f\u2028\udfff\ud800\ud83d\ude00')).toBe('"\\u0000\\u001f\\b\\f\\n\\r\\t\\\"\\\\/\u007f\u2028\\udfff\\ud800\ud83d\ude00"');
  });

  it('skips what JSON.stringify skips', () => {
    expect(canonicalStringify({ a: undefined, b: () => 1, c: Symbol('x'), d: 1 })).toBe('{"d":1}');
    // eslint-disable-next-line no-sparse-arrays
    expect(canonicalStringify([undefined, () => 1, , 2])).toBe('[null,null,null,2]');
    expect(canonicalStringify(undefined)).toBeUndefined();
    expect(canonicalStringify({ toJSON: () => ({ z: 1, y: 2 }) })).toBe('{"y":2,"z":1}');
    expect(canonicalStringify(new Date(0))).toBe('"1970-01-01T00:00:00.000Z"');
    expect(canonicalStringify([new Number(1), new String('s'), new Boolean(false)])).toBe('[1,"s",false]');
    expect(() => canonicalStringify(1n)).toThrow(TypeError);
    const cycle: any = { a: {} }; cycle.a.b = cycle;
    expect(() => canonicalStringify(cycle)).toThrow(TypeError);
  });

  it('keeps own __proto__ members that JSON.parse creates', () => {
    expect(canonicalStringify(JSON.parse('{"__proto__":{"x":1},"a":1}'))).toBe('{"__proto__":{"x":1},"a":1}');
  });

  it('agrees with JSON.stringify whenever keys are already in canonical order', () => {
    let seed = 7;
    const rand = () => (seed = (seed * 1103515245 + 12345) % 2147483648) / 2147483648;
    const make = (depth: number): any => {
      const r = rand();
      if (depth === 0 || r < 0.3) return [null, true, false, rand() * 1e6 - 5e5, 'x' + rand(), ''][Math.floor(rand() * 6)];
      if (r < 0.6) return Array.from({ length: Math.floor(rand() * 4) }, () => make(depth - 1));
      const keys = Array.from({ length: Math.floor(rand() * 4) }, () => 'k' + Math.floor(rand() * 50)).sort();
      return Object.fromEntries(keys.map(key => [key, make(depth - 1)]));
    };
    for (let i = 0; i < 500; i++) {
      const value = make(4);
      expect(canonicalStringify(value)).toBe(JSON.stringify(value));
    }
  });

  it('hashes the UTF-8 bytes', () => {
    expect(canonicalHash({})).toBe('44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a');
    const value = { b: [1, 2], a: 'é😀' };
    expect(canonicalHash(value)).toBe(sha256Hex(new TextEncoder().encode('{"a":"é😀","b":[1,2]}')));
    expect(() => canonicalHash(undefined)).toThrow(TypeError);
  });

  it('hashes in pieces exactly what the whole serialization hashes', () => {
    // The streaming hash (no whole-document string) must equal the hash of
    // canonicalStringify for every value shape, across piece boundaries.
    const odd = { toJSON: (key: string) => ({ key, n: new Number(3) }) };
    const big = Array.from({ length: 4000 }, (_, i) => ({
      id: `L${i}`, t: i / 30, v: i % 7 ? i * 1.5 : -0, nan: NaN, skip: undefined, fn: () => 1,
      text: i % 3 ? '· — “é😀”' : 'lone \ud800 end', list: [undefined, null, Infinity, () => 2, Symbol('s')], nested: { b: 1, a: [odd] }
    }));
    for (const value of [big, { z: big, a: 'x'.repeat(70_000), m: '😀'.repeat(40_000) }, [1, 'two', null, true], 'top', 12, null, odd, new String('boxed')]) {
      expect(canonicalHash(value)).toBe(sha256Hex(new TextEncoder().encode(canonicalStringify(value)!)));
    }
    const cycle: any = { a: 1 }; cycle.self = cycle;
    expect(() => canonicalHash(cycle)).toThrow(TypeError);
    expect(() => canonicalHash({ n: 1n })).toThrow(TypeError);
    expect(() => canonicalHash(() => 1)).toThrow(TypeError);
  });
});

describe('canonicalProject', () => {
  it('uses the composition fields of legacy/core/compositions.ts', () => {
    expect([...CANONICAL_COMP_FIELDS]).toEqual([...COMP_FIELDS]);
  });

  it('stores the open composition like the others', () => {
    const project = { id: 'P', name: 'Project', compId: 'C1', compName: 'Main', w: 10, fps: 30, layers: [{ id: 'a' }], assets: {}, comps: { C2: { id: 'C2', w: 5 } } };
    expect(canonicalStringify(canonicalProject(project))).toBe(
      '{"assets":{},"compId":"C1","comps":{"C1":{"fps":30,"id":"C1","layers":[{"id":"a"}],"name":"Main","w":10},"C2":{"id":"C2","w":5}},"id":"P","name":"Project"}');
    expect(canonicalStringify(realizeProject(canonicalProject(project)))).toBe(canonicalStringify(project));
    expect(canonicalProjectHash(project)).toBe(canonicalHash(canonicalProject(project)));
  });

  it('leaves values it cannot canonicalize unchanged', () => {
    for (const value of [5, [1], { w: 1 }, { compId: '', w: 1 }, { compId: 7 }, { compId: 'C', comps: [1] }]) {
      expect(canonicalProject(value)).toBe(value);
    }
    expect(canonicalStringify(canonicalProject({ compId: 'C', w: 2, comps: { C: { stale: true } } }))).toBe('{"compId":"C","comps":{"C":{"id":"C","w":2}}}');
    expect(canonicalStringify(realizeProject({ compId: 'C', comps: { C: { id: 'C', name: 'N', w: 3, extra: 1 } } }))).toBe('{"compId":"C","compName":"N","comps":{},"w":3}');
  });

  it('does not depend on which composition is open, beyond the pointer', () => {
    const project = { id: 'P', compId: 'C1', compName: 'Main', w: 10, h: 10, fps: 30, dur: 2, layers: [], markers: [], work: [0, 2], comps: { C2: { id: 'C2', name: 'Two', w: 5, h: 5, fps: 24, dur: 1, layers: [], markers: [], work: [0, 1] } } };
    const a = canonicalProject(project);
    const b = canonicalProject(rotateProject(project, 'C2'));
    expect({ ...b, compId: 'C1' }).toEqual({ ...a });
  });

  it('round-trips every corpus fixture and matches the recorded engine vectors', () => {
    const vectors = JSON.parse(readFileSync(path.join(engineTests, 'vectors/canonical-vectors.json'), 'utf8'));
    const recorded = new Map<string, string>(vectors.projects.map((p: any) => [p.name, p.hash]));
    const dir = path.join(engineTests, 'fixtures/corpus');
    const names = readdirSync(dir).filter(name => name.endsWith('.json'));
    expect(names.length).toBeGreaterThan(30);
    for (const name of names) {
      const project = JSON.parse(readFileSync(path.join(dir, name), 'utf8'));
      const canonical = canonicalProject(project);
      expect(canonicalStringify(realizeProject(canonical))).toBe(canonicalStringify(project));
      expect(canonicalHash(canonical)).toBe(recorded.get(name));
    }
  });
});
