// Runs under `bun test` and vitest. engine/crates/pm-model (tests/patch_binary.rs)
// decodes, re-encodes, and applies the same golden vectors in Rust.
import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import {
  buildPatchBinaryVectors, hex, randomPair, serializeVectors, VECTORS_FILE, type PatchBinaryVectors,
} from '../../../../engine/crates/pm-model/tests/vectors/generate-patch-binary-vectors';
import { decodePatchOps, encodePatchOps, isCanonicalPatchBody, PatchBinaryError } from './patch-binary';
import { diffOps, type PatchOp } from './patch-ids';

const unhex = (text: string) => Uint8Array.from(text.match(/../g) ?? [], (byte) => parseInt(byte, 16));
const committed = () => readFileSync(VECTORS_FILE, 'utf8');
/** JSON form for comparison: what the vectors store. */
const jsonForm = (ops: PatchOp[]) => JSON.parse(JSON.stringify(ops));

describe('canonical binary patches (canonicalV1)', () => {
  it('matches the committed cross-language vectors byte for byte', () => {
    // Regenerate with: bun engine/crates/pm-model/tests/vectors/generate-patch-binary-vectors.ts
    expect(serializeVectors(buildPatchBinaryVectors())).toBe(committed());
  });

  it('decodes every vector to its operations and re-encodes to the same bytes', () => {
    const vectors = JSON.parse(committed()) as PatchBinaryVectors;
    for (const c of [...vectors.cases, ...vectors.applyCases]) {
      const bytes = unhex(c.hex);
      const ops = decodePatchOps(bytes);
      expect(jsonForm(ops), c.name).toEqual(c.ops);
      expect(hex(encodePatchOps(ops)), c.name).toBe(c.hex);
      expect(isCanonicalPatchBody(bytes), c.name).toBe(true);
    }
  });

  it('rejects every invalid body', () => {
    const vectors = JSON.parse(committed()) as PatchBinaryVectors;
    expect(vectors.invalid.length).toBeGreaterThan(20);
    for (const c of vectors.invalid) {
      expect(() => decodePatchOps(unhex(c.hex)), c.name).toThrow(PatchBinaryError);
      expect(isCanonicalPatchBody(unhex(c.hex)), c.name).toBe(false);
    }
  });

  it('round-trips every operation kind, -0 and 2^32 path numbers, and lone surrogates', () => {
    const ops: PatchOp[] = [
      { path: ['comps', 'c1', 'layers', 3, 'p'], exists: true, value: { b: 1, a: [true, null, 'x\ud800'] } },
      { path: ['gone'], exists: false },
      { path: ['undef'], exists: true },
      { op: 'order', path: ['l'], ids: ['b', '\udc00', '😀'] },
      { op: 'insert', path: [], index: 7, value: { id: 'n', v: 1e21 } },
      { op: 'delete', path: [-0, 2 ** 32, 'k\udfff'], id: 'z' },
    ];
    const bytes = encodePatchOps(ops);
    expect(Array.from(bytes.subarray(0, 4))).toEqual([0x50, 0x4d, 0x50, 0x01]);
    const back = decodePatchOps(bytes);
    expect(back).toEqual(ops);
    expect(Object.is((back[5] as { path: number[] }).path[0], -0)).toBe(true);
    // Values are canonical JSON: sorted keys.
    expect(new TextDecoder().decode(bytes)).toContain('{"a":[true,null,"x\\ud800"],"b":1}');
    for (let end = 0; end < bytes.length; end++) expect(() => decodePatchOps(bytes.subarray(0, end))).toThrow(PatchBinaryError);
  });

  it('accepts any JSON text on decode but only canonical bodies are canonical', () => {
    const canonical = encodePatchOps([{ path: ['a'], exists: true, value: { a: 1, b: 2 } }]);
    const text = new TextEncoder().encode('{"b":2,"a":1}');
    const loose = new Uint8Array(canonical.length);
    loose.set(canonical.subarray(0, canonical.length - text.length));
    loose.set(text, canonical.length - text.length);
    expect(decodePatchOps(loose)).toEqual(decodePatchOps(canonical));
    expect(isCanonicalPatchBody(loose)).toBe(false);
    expect(isCanonicalPatchBody(canonical)).toBe(true);
  });

  it('refuses operations the format cannot carry', () => {
    const bad: PatchOp[][] = [
      [{ path: [1.5], exists: false }],
      [{ path: [-1], exists: false }],
      [{ path: [NaN], exists: false }],
      [{ path: [true as any], exists: false }],
      [{ path: ['f'], exists: true, value: () => 1 }],
      [{ op: 'insert', path: [], index: 0, value: { name: 'no id' } }],
      [{ op: 'insert', path: [], index: -1, value: { id: 'a' } }],
      [{ op: 'order', path: [], ids: [1 as any] }],
    ];
    for (const ops of bad) expect(() => encodePatchOps(ops), JSON.stringify(ops)).toThrow(PatchBinaryError);
  });

  it('round-trips generated operations on random documents', () => {
    for (let seed = 0; seed < 1000; seed++) {
      const [before, after] = randomPair(0x0b1e0000 + seed);
      const ops = diffOps(before, after);
      const bytes = encodePatchOps(ops);
      expect(jsonForm(decodePatchOps(bytes)), `seed ${seed}`).toEqual(jsonForm(ops));
      expect(isCanonicalPatchBody(bytes)).toBe(true);
    }
  });
});
