import { describe, expect, it } from 'vitest';
import { DIGEST_CHUNK, TextDigest, digestJSON, digestText, serializeInSlices } from './json-slices';

const immediate = () => Promise.resolve();

async function sliced(value: unknown): Promise<string> {
  const pieces: string[] = [];
  await serializeInSlices(value, text => { pieces.push(text); }, immediate);
  return pieces.join('');
}

/** A small deterministic generator (mulberry32). */
function random(seed: number) {
  return () => {
    seed |= 0; seed = seed + 0x6d2b79f5 | 0;
    let t = Math.imul(seed ^ seed >>> 15, 1 | seed);
    t = t + Math.imul(t ^ t >>> 7, 61 | t) ^ t;
    return ((t ^ t >>> 14) >>> 0) / 4294967296;
  };
}

class WithProtoJSON { constructor(readonly x: number) {} toJSON(key: string) { return { tagged: key, x: this.x }; } }
class Plain { a = 1; b = 'two'; }

function value(next: () => number, depth: number): any {
  const pick = Math.floor(next() * (depth > 9 ? 9 : 16));
  const large = depth < 3 && next() < 0.2;
  switch (pick) {
    case 0: return null;
    case 1: return next() < 0.5;
    case 2: return [0, -0, 1.5, -2e-300, 1e21, NaN, Infinity, -Infinity, 2 ** 53][Math.floor(next() * 9)];
    case 3: return ['', 'plain', 'quote"back\\slash', 'line\nbreak ', '😀 pair', 'lone \ud800 high', 'lone \udc00 low', 'é ü 中'][Math.floor(next() * 8)];
    case 4: return undefined;
    case 5: return () => 1;
    case 6: return Symbol('s');
    case 7: return Math.floor(next() * 1000);
    case 8: return 'x'.repeat(Math.floor(next() * 40));
    case 9: case 10: case 11: {
      const length = large ? 70 + Math.floor(next() * 120) : Math.floor(next() * 4);
      const array: any[] = [];
      for (let i = 0; i < length; i++) array.push(value(next, depth + 1));
      if (next() < 0.1) array[length + 2] = 'after a hole';
      return array;
    }
    case 12: case 13: {
      const members = large ? 70 + Math.floor(next() * 60) : Math.floor(next() * 4);
      const object: any = {};
      for (let i = 0; i < members; i++) object[next() < 0.2 ? String(i) : `k${i}_${Math.floor(next() * 9)}`] = value(next, depth + 1);
      return object;
    }
    case 14: return [new Date(Math.floor(next() * 1e12)), new WithProtoJSON(Math.floor(next() * 9)), new Plain(),
      Object(5), Object('boxed'), Object(false), { toJSON() { return this; }, kept: 1 },
      { toJSON: (key: string) => ({ inner: { toJSON: () => `again:${key}` } }) }, new Map([[1, 2]]), new Uint8Array([1, 2, 3])][Math.floor(next() * 10)];
    default: return { nested: value(next, depth + 1), list: [value(next, depth + 2)] };
  }
}

describe('serializeInSlices', () => {
  it('writes exactly what JSON.stringify writes', async () => {
    for (let seed = 1; seed <= 400; seed++) {
      const next = random(seed);
      const wide: any[] = Array.from({ length: 300 }, () => value(next, 6));
      if (seed % 3 === 0) wide[150] = new WithProtoJSON(seed);
      if (seed % 5 === 0) delete wide[20];
      const input = { v: '1.0.0', proj: value(next, 0), deep: [[[[[[[[value(next, 0)]]]]]]]], wide: [[[[[[wide]]]]]], wideObject: [[[[[[Object.fromEntries([...wide].map((item, index) => [`m${index}`, item]))]]]]]] };
      expect(await sliced(input)).toBe(JSON.stringify(input));
    }
  });

  it('serializes a large document in slices that each stay short', async () => {
    const layers = Array.from({ length: 40 }, (_, index) => ({
      id: `layer-${index}`,
      p: Object.fromEntries(['x', 'y', 'r', 's', 'o'].map(name => [name, Array.from({ length: 2000 }, (_, k) => ({ t: k / 24, v: Math.sin(k + index), eo: [0.33, 0], ei: [0.67, 1] }))])),
    }));
    const project = { v: '1.0.0', proj: { id: 'p', w: 1920, h: 1080, layers } };
    let longest = 0, last = performance.now();
    const pause = async () => { const now = performance.now(); longest = Math.max(longest, now - last); await Promise.resolve(); last = performance.now(); };
    const pieces: string[] = [];
    await serializeInSlices(project, text => { pieces.push(text); }, pause);
    expect(pieces.join('')).toBe(JSON.stringify(project));
    expect(pieces.length).toBeGreaterThan(10);
    // Generous for loaded CI machines; a whole-document stringify of this takes far longer.
    expect(longest).toBeLessThan(60);
  });

  it('keeps a slice short where a container of few members deep in the document holds many values, or array elements are large', async () => {
    const numbers = (n: number) => Array.from({ length: n }, (_, i) => i / 7);
    const deep = (value: unknown) => [[[[[[[value]]]]]]];
    const project = {
      held: deep({ points: numbers(1_500_000), name: 'path' }),
      nested: deep([1, numbers(1_500_000)]),
      elements: deep(Array.from({ length: 300 }, (_, i) => ({ id: i, samples: numbers(20_000) }))),
    };
    let longest = 0, last = performance.now();
    const pause = async () => { const now = performance.now(); longest = Math.max(longest, now - last); await Promise.resolve(); last = performance.now(); };
    const pieces: string[] = [];
    await serializeInSlices(project, text => { pieces.push(text); }, pause);
    expect(pieces.join('')).toBe(JSON.stringify(project));
    // Generous for loaded CI machines; one native call over each array takes far longer.
    expect(longest).toBeLessThan(60);
  });

  it('matches the top-level cases, toJSON keys, and errors', async () => {
    for (const input of [null, 1, 'text', [undefined, () => 1], new Date(0), Object(3), { toJSON: () => [1, { toJSON: (k: string) => k }] }]) {
      expect(await sliced(input)).toBe(JSON.stringify(input));
    }
    await expect(sliced(undefined)).rejects.toThrow(TypeError);
    const cycle: any = { a: [{}] }; cycle.a[0].back = cycle;
    expect(() => JSON.stringify(cycle)).toThrow(TypeError);
    await expect(sliced(cycle)).rejects.toThrow(TypeError);
    const deepCycle: any = [[[[[[[[{}]]]]]]]]; deepCycle[0][0][0][0][0][0][0][0].up = deepCycle[0];
    await expect(sliced(deepCycle)).rejects.toThrow(TypeError);
    await expect(sliced({ big: 1n })).rejects.toThrow(TypeError);
    // A shared (not circular) object is written each time it appears.
    const shared = { s: 1 };
    const twice = { a: shared, b: [shared, shared] };
    expect(await sliced(twice)).toBe(JSON.stringify(twice));
  });
});

describe('JSON digests', () => {
  it('is the same for a whole string, pieces of it, and the sliced serializer', async () => {
    const next = random(7);
    const input = { v: '1.0.0', proj: Array.from({ length: 30 }, () => value(next, 0)) };
    const text = JSON.stringify(input);
    const whole = await digestText(text, immediate);
    const pieces = new TextDigest();
    for (let offset = 0; offset < text.length; offset += 777) await pieces.push(text.slice(offset, offset + 777));
    expect(await pieces.finish()).toBe(whole);
    expect(await digestJSON(input, immediate)).toEqual({ digest: whole, length: text.length });
    expect(await digestText(text + ' ', immediate)).not.toBe(whole);
  });

  it('never cuts a surrogate pair at a chunk boundary', async () => {
    // A pair straddling the boundary, against the same length with U+FFFD:
    // a lossy cut would encode both as the same bytes.
    const pair = 'a'.repeat(DIGEST_CHUNK - 1) + '😀' + 'tail';
    const replaced = 'a'.repeat(DIGEST_CHUNK - 1) + '��' + 'tail';
    expect(await digestText(pair, immediate)).not.toBe(await digestText(replaced, immediate));
    const streamed = new TextDigest();
    for (let offset = 0; offset < pair.length; offset += 1 << 20) await streamed.push(pair.slice(offset, offset + (1 << 20)));
    expect(await streamed.finish()).toBe(await digestText(pair, immediate));
  });
});
