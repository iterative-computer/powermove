import { createHash } from 'node:crypto';
import { describe, expect, it } from 'vitest';
import { canonicalStringify } from '../../../shared/canonical-json';
import { digestInstructions, type DigestInstruction } from '../../../shared/document-digest';
import { ConsistencyHash } from './consistency-hash';

const hash = (value: any, prefix = '') => createHash('sha256').update(prefix).update(canonicalStringify(value)!).digest('hex');
const streamed = async (value: any, prefix = '') => hash(value, prefix);
const batch = async (instructions: DigestInstruction[]) => digestInstructions(instructions, hash);

function reference(value: any, depth = 0): string {
  const container = value !== null && typeof value === 'object';
  const bulk = Array.isArray(value) && value.length > 64 && !value.every(item => item && typeof item === 'object' && typeof item.id === 'string');
  if (depth >= 6 || !container || bulk) return hash(value, 'v');
  if (Array.isArray(value)) return hash(['a', value.map(child => reference(child, depth + 1))]);
  return hash(['o', Object.keys(value).sort().map(key => [key, reference(value[key], depth + 1)])]);
}

describe('ConsistencyHash async batches', () => {
  it('matches the existing synchronous digest including depth, bulk and UTF-16 ordering', async () => {
    const values = [null, 3, 'é😀\ud800', [], {}, { z: [1, 2], a: { '\uffff': 1, '😀': 2 } },
      Array.from({ length: 100 }, (_, i) => i), Array.from({ length: 100 }, (_, i) => ({ id: String(i), a: i })),
      { a: { b: { c: { d: { e: { f: { g: [1, 2, 3] } } } } } } },
      JSON.parse('{"__proto__":{"x":1},"a":2}')];
    for (const value of values) {
      const expected = new ConsistencyHash().digest(value);
      expect(await new ConsistencyHash().digestAsync(value, streamed, batch)).toBe(expected);
      expect(await new ConsistencyHash().digestAsync(value, streamed)).toBe(expected);
    }
    expect(await new ConsistencyHash().digestAsync(undefined, streamed, batch)).toBe('');
  });

  it('does not repopulate invalidated caches after an awaited hash finishes', async () => {
    for (const batched of [false, true]) {
      const integrity = new ConsistencyHash(), root = { a: { n: 1 } };
      let release!: () => void, started!: () => void;
      const gate = new Promise<void>(resolve => { release = resolve; });
      const paused = new Promise<void>(resolve => { started = resolve; });
      let first = true;
      const pause = async () => { if (first) { first = false; started(); await gate; } };
      const slowHash = async (value: any, prefix = '') => { const digest = hash(value, prefix); await pause(); return digest; };
      const slowBatch = async (instructions: DigestInstruction[]) => { const hashes = digestInstructions(instructions, hash); await pause(); return hashes; };
      const pending = integrity.digestAsync(root, slowHash, batched ? slowBatch : undefined);
      await paused;
      integrity.invalidate(root, [{ path: ['a', 'n'], exists: true, value: 2 }]);
      root.a.n = 2;
      release();
      await pending;
      expect(await integrity.digestAsync(root, streamed, batch)).toBe(new ConsistencyHash().digest(root));
      let calls = 0;
      expect(await integrity.digestAsync(root, async () => { calls++; throw new Error('cache miss'); }, async () => { calls++; throw new Error('cache miss'); })).toBe(reference(root));
      expect(calls).toBe(0);
    }
  });

  it('streams large leaves and parent metadata after resolving pending references', async () => {
    const root = { a: '😀'.repeat(20_000), b: Array.from({ length: 5000 }, (_, i) => ({ id: String(i), n: i })) };
    let streams = 0, batches = 0;
    const got = await new ConsistencyHash().digestAsync(root, async (value, prefix) => { streams++; return hash(value, prefix); }, async instructions => { batches++; return digestInstructions(instructions, hash); });
    expect(got).toBe(reference(root));
    expect(streams).toBeGreaterThanOrEqual(2);
    expect(batches).toBeGreaterThan(1);
  });

  it('keeps batch calls below 3000 for the 6.2 MB / 40k object fixture and warms the root', async () => {
    const root = Array.from({ length: 40_000 }, (_, i) => ({ id: `L${i}`, n: i, text: 'é😀'.repeat(20) }));
    let calls = 0, streams = 0;
    const integrity = new ConsistencyHash();
    const got = await integrity.digestAsync(root, async (value, prefix) => { streams++; return hash(value, prefix); }, async instructions => {
      calls++;
      expect(instructions.length).toBeLessThanOrEqual(128);
      expect(Buffer.byteLength(JSON.stringify(instructions))).toBeLessThanOrEqual(256 * 1024);
      return digestInstructions(instructions, hash);
    });
    expect(got).toBe(reference(root));
    expect(calls).toBeLessThan(3000);
    expect(streams).toBe(1);
    const before = calls;
    expect(await integrity.digestAsync(root, streamed, async instructions => { calls++; return digestInstructions(instructions, hash); })).toBe(got);
    expect(calls).toBe(before);
    expect(integrity.reused).toBe(1);
  }, 20_000);

  it('rejects incomplete host batches without caching their results', async () => {
    const root = { a: 1 };
    const integrity = new ConsistencyHash();
    await expect(integrity.digestAsync(root, streamed, async () => [])).rejects.toThrow(/batch response/);
    expect(await integrity.digestAsync(root, streamed, batch)).toBe(reference(root));
  });
});
