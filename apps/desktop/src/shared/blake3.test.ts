import { describe, expect, it } from 'vitest';

import { Blake3, blake3Hex, CHUNK_LEN } from './blake3';
import vectors from './blake3-vectors.json';

/* Reference digests from the official BLAKE3 implementation (the Rust crate
   compiled to WebAssembly, `blake3-wasm`), over the official test-vector
   input: byte i is i % 251. The empty digest is the specification's. */
const input = (length: number): Uint8Array => {
  const bytes = new Uint8Array(length);
  for (let i = 0; i < length; i++) bytes[i] = i % 251;
  return bytes;
};

const entries = Object.entries(vectors as Record<string, string>).map(([length, hex]) => [Number(length), hex] as const);

describe('BLAKE3', () => {
  it('matches the reference digest for every vector length in one update', () => {
    expect(blake3Hex(new Uint8Array(0))).toBe('af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262');
    for (const [length, hex] of entries) expect(blake3Hex(input(length)), `length ${length}`).toBe(hex);
  });

  it('gives the same digest however the input is split, aligned or not', () => {
    let seed = 7;
    const random = () => { seed = (seed * 1103515245 + 12345) & 0x7fffffff; return seed; };
    for (const [length, hex] of entries) {
      if (length > 200_000) continue;
      const bytes = input(length);
      for (let trial = 0; trial < 4; trial++) {
        const hasher = new Blake3();
        for (let at = 0; at < length;) {
          const step = Math.min(length - at, 1 + (random() % (trial === 0 ? 70 : 3000)));
          // Copy into a buffer at an odd offset so the word fast path is off.
          const piece = trial % 2 ? (() => { const holder = new Uint8Array(step + 1); holder.set(bytes.subarray(at, at + step), 1); return holder.subarray(1); })()
            : bytes.subarray(at, at + step);
          hasher.update(piece);
          at += step;
        }
        expect(hasher.hex(), `length ${length} trial ${trial}`).toBe(hex);
        expect(hasher.length).toBe(length);
      }
    }
  });

  it('keeps going after a digest', () => {
    const bytes = input(5121);
    const hasher = new Blake3().update(bytes.subarray(0, 2049));
    expect(hasher.hex()).toBe((vectors as Record<string, string>)['2049']);
    hasher.update(bytes.subarray(2049));
    expect(hasher.hex()).toBe((vectors as Record<string, string>)['5121']);
  });

  it('assembles subtree chaining values into the single-threaded digest', () => {
    const length = 4194304 + 333;
    const bytes = input(length);
    for (const segmentChunks of [1, 2, 64, 1024]) {
      const segment = segmentChunks * CHUNK_LEN;
      const hasher = new Blake3();
      let at = 0;
      // Every segment that does not hold the last byte is a subtree.
      while (length - at > segment) {
        const part = new Blake3(at / CHUNK_LEN).update(bytes.subarray(at, at + segment));
        hasher.pushSubtree(part.subtreeChainingValue(), segmentChunks);
        at += segment;
      }
      hasher.update(bytes.subarray(at));
      expect(hasher.hex(), `segments of ${segmentChunks} chunks`).toBe((vectors as Record<string, string>)[String(length)]);
      expect(hasher.length).toBe(length);
    }
  });

  it('refuses a subtree off its boundary or with bytes buffered', () => {
    const cv = new Uint32Array(8);
    expect(() => new Blake3().pushSubtree(cv, 3)).toThrow(/power-of-two/);
    expect(() => new Blake3(1).pushSubtree(cv, 2)).toThrow(/boundary/);
    expect(() => new Blake3().update(new Uint8Array(1)).pushSubtree(cv, 1)).toThrow(/boundary/);
    expect(() => new Blake3().update(new Uint8Array(100)).subtreeChainingValue()).toThrow(/whole chunk/);
  });
});
