/*
 * BLAKE3 (the plain hash mode, 32-byte output) for the media store.
 *
 * Content identity across Powermove is BLAKE3 (plan section 13, probe item
 * 6; `pm-media::hash`). Main needs the same digest without loading an
 * engine addon, so this is a straight implementation of the specification
 * (the reference implementation's chunk and tree logic) with the round
 * function unrolled. It is verified against the reference digests in
 * `blake3.test.ts`, which come from the official implementation.
 *
 * Subtrees: a run of 2^k whole chunks that starts at a multiple of 2^k is a
 * complete subtree whose chaining value can be computed on its own
 * (`subtreeChainingValue`) and pushed into another hasher
 * (`pushSubtree`). That is how a large file is hashed by several workers
 * and still yields the single-threaded digest.
 */

export const CHUNK_LEN = 1024;
export const BLOCK_LEN = 64;
const CHUNK_START = 1;
const CHUNK_END = 2;
const PARENT = 4;
const ROOT = 8;
const TWO_32 = 0x1_0000_0000;

const IV = new Uint32Array([0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19]);
const LITTLE_ENDIAN = new Uint8Array(new Uint32Array([1]).buffer)[0] === 1;

/**
 * The compression function. Reads 8 chaining-value words at `cv[cvo]` and 16
 * message words at `m[mo]`; writes 8 words (or 16 when `full`) at `out[oo]`.
 * `out` may alias `cv` (the chaining value is read before anything is
 * written).
 */
function compress(cv: Uint32Array, cvo: number, m: Uint32Array, mo: number, counter: number, blockLen: number, flags: number,
  out: Uint32Array, oo: number, full: boolean): void {
  const m0 = m[mo]! | 0, m1 = m[mo + 1]! | 0, m2 = m[mo + 2]! | 0, m3 = m[mo + 3]! | 0;
  const m4 = m[mo + 4]! | 0, m5 = m[mo + 5]! | 0, m6 = m[mo + 6]! | 0, m7 = m[mo + 7]! | 0;
  const m8 = m[mo + 8]! | 0, m9 = m[mo + 9]! | 0, m10 = m[mo + 10]! | 0, m11 = m[mo + 11]! | 0;
  const m12 = m[mo + 12]! | 0, m13 = m[mo + 13]! | 0, m14 = m[mo + 14]! | 0, m15 = m[mo + 15]! | 0;
  const c0 = cv[cvo]! | 0, c1 = cv[cvo + 1]! | 0, c2 = cv[cvo + 2]! | 0, c3 = cv[cvo + 3]! | 0;
  const c4 = cv[cvo + 4]! | 0, c5 = cv[cvo + 5]! | 0, c6 = cv[cvo + 6]! | 0, c7 = cv[cvo + 7]! | 0;
  let v0 = c0, v1 = c1, v2 = c2, v3 = c3, v4 = c4, v5 = c5, v6 = c6, v7 = c7;
  let v8 = 0x6a09e667 | 0, v9 = 0xbb67ae85 | 0, v10 = 0x3c6ef372 | 0, v11 = 0xa54ff53a | 0;
  let v12 = counter | 0, v13 = Math.floor(counter / TWO_32) | 0, v14 = blockLen | 0, v15 = flags | 0;
    // round 1
    v0 = (v0 + v4 + m0) | 0; v12 ^= v0; v12 = ((v12 >>> 16) | (v12 << 16));
    v8 = (v8 + v12) | 0; v4 ^= v8; v4 = ((v4 >>> 12) | (v4 << 20));
    v0 = (v0 + v4 + m1) | 0; v12 ^= v0; v12 = ((v12 >>> 8) | (v12 << 24));
    v8 = (v8 + v12) | 0; v4 ^= v8; v4 = ((v4 >>> 7) | (v4 << 25));
    v1 = (v1 + v5 + m2) | 0; v13 ^= v1; v13 = ((v13 >>> 16) | (v13 << 16));
    v9 = (v9 + v13) | 0; v5 ^= v9; v5 = ((v5 >>> 12) | (v5 << 20));
    v1 = (v1 + v5 + m3) | 0; v13 ^= v1; v13 = ((v13 >>> 8) | (v13 << 24));
    v9 = (v9 + v13) | 0; v5 ^= v9; v5 = ((v5 >>> 7) | (v5 << 25));
    v2 = (v2 + v6 + m4) | 0; v14 ^= v2; v14 = ((v14 >>> 16) | (v14 << 16));
    v10 = (v10 + v14) | 0; v6 ^= v10; v6 = ((v6 >>> 12) | (v6 << 20));
    v2 = (v2 + v6 + m5) | 0; v14 ^= v2; v14 = ((v14 >>> 8) | (v14 << 24));
    v10 = (v10 + v14) | 0; v6 ^= v10; v6 = ((v6 >>> 7) | (v6 << 25));
    v3 = (v3 + v7 + m6) | 0; v15 ^= v3; v15 = ((v15 >>> 16) | (v15 << 16));
    v11 = (v11 + v15) | 0; v7 ^= v11; v7 = ((v7 >>> 12) | (v7 << 20));
    v3 = (v3 + v7 + m7) | 0; v15 ^= v3; v15 = ((v15 >>> 8) | (v15 << 24));
    v11 = (v11 + v15) | 0; v7 ^= v11; v7 = ((v7 >>> 7) | (v7 << 25));
    v0 = (v0 + v5 + m8) | 0; v15 ^= v0; v15 = ((v15 >>> 16) | (v15 << 16));
    v10 = (v10 + v15) | 0; v5 ^= v10; v5 = ((v5 >>> 12) | (v5 << 20));
    v0 = (v0 + v5 + m9) | 0; v15 ^= v0; v15 = ((v15 >>> 8) | (v15 << 24));
    v10 = (v10 + v15) | 0; v5 ^= v10; v5 = ((v5 >>> 7) | (v5 << 25));
    v1 = (v1 + v6 + m10) | 0; v12 ^= v1; v12 = ((v12 >>> 16) | (v12 << 16));
    v11 = (v11 + v12) | 0; v6 ^= v11; v6 = ((v6 >>> 12) | (v6 << 20));
    v1 = (v1 + v6 + m11) | 0; v12 ^= v1; v12 = ((v12 >>> 8) | (v12 << 24));
    v11 = (v11 + v12) | 0; v6 ^= v11; v6 = ((v6 >>> 7) | (v6 << 25));
    v2 = (v2 + v7 + m12) | 0; v13 ^= v2; v13 = ((v13 >>> 16) | (v13 << 16));
    v8 = (v8 + v13) | 0; v7 ^= v8; v7 = ((v7 >>> 12) | (v7 << 20));
    v2 = (v2 + v7 + m13) | 0; v13 ^= v2; v13 = ((v13 >>> 8) | (v13 << 24));
    v8 = (v8 + v13) | 0; v7 ^= v8; v7 = ((v7 >>> 7) | (v7 << 25));
    v3 = (v3 + v4 + m14) | 0; v14 ^= v3; v14 = ((v14 >>> 16) | (v14 << 16));
    v9 = (v9 + v14) | 0; v4 ^= v9; v4 = ((v4 >>> 12) | (v4 << 20));
    v3 = (v3 + v4 + m15) | 0; v14 ^= v3; v14 = ((v14 >>> 8) | (v14 << 24));
    v9 = (v9 + v14) | 0; v4 ^= v9; v4 = ((v4 >>> 7) | (v4 << 25));
    // round 2
    v0 = (v0 + v4 + m2) | 0; v12 ^= v0; v12 = ((v12 >>> 16) | (v12 << 16));
    v8 = (v8 + v12) | 0; v4 ^= v8; v4 = ((v4 >>> 12) | (v4 << 20));
    v0 = (v0 + v4 + m6) | 0; v12 ^= v0; v12 = ((v12 >>> 8) | (v12 << 24));
    v8 = (v8 + v12) | 0; v4 ^= v8; v4 = ((v4 >>> 7) | (v4 << 25));
    v1 = (v1 + v5 + m3) | 0; v13 ^= v1; v13 = ((v13 >>> 16) | (v13 << 16));
    v9 = (v9 + v13) | 0; v5 ^= v9; v5 = ((v5 >>> 12) | (v5 << 20));
    v1 = (v1 + v5 + m10) | 0; v13 ^= v1; v13 = ((v13 >>> 8) | (v13 << 24));
    v9 = (v9 + v13) | 0; v5 ^= v9; v5 = ((v5 >>> 7) | (v5 << 25));
    v2 = (v2 + v6 + m7) | 0; v14 ^= v2; v14 = ((v14 >>> 16) | (v14 << 16));
    v10 = (v10 + v14) | 0; v6 ^= v10; v6 = ((v6 >>> 12) | (v6 << 20));
    v2 = (v2 + v6 + m0) | 0; v14 ^= v2; v14 = ((v14 >>> 8) | (v14 << 24));
    v10 = (v10 + v14) | 0; v6 ^= v10; v6 = ((v6 >>> 7) | (v6 << 25));
    v3 = (v3 + v7 + m4) | 0; v15 ^= v3; v15 = ((v15 >>> 16) | (v15 << 16));
    v11 = (v11 + v15) | 0; v7 ^= v11; v7 = ((v7 >>> 12) | (v7 << 20));
    v3 = (v3 + v7 + m13) | 0; v15 ^= v3; v15 = ((v15 >>> 8) | (v15 << 24));
    v11 = (v11 + v15) | 0; v7 ^= v11; v7 = ((v7 >>> 7) | (v7 << 25));
    v0 = (v0 + v5 + m1) | 0; v15 ^= v0; v15 = ((v15 >>> 16) | (v15 << 16));
    v10 = (v10 + v15) | 0; v5 ^= v10; v5 = ((v5 >>> 12) | (v5 << 20));
    v0 = (v0 + v5 + m11) | 0; v15 ^= v0; v15 = ((v15 >>> 8) | (v15 << 24));
    v10 = (v10 + v15) | 0; v5 ^= v10; v5 = ((v5 >>> 7) | (v5 << 25));
    v1 = (v1 + v6 + m12) | 0; v12 ^= v1; v12 = ((v12 >>> 16) | (v12 << 16));
    v11 = (v11 + v12) | 0; v6 ^= v11; v6 = ((v6 >>> 12) | (v6 << 20));
    v1 = (v1 + v6 + m5) | 0; v12 ^= v1; v12 = ((v12 >>> 8) | (v12 << 24));
    v11 = (v11 + v12) | 0; v6 ^= v11; v6 = ((v6 >>> 7) | (v6 << 25));
    v2 = (v2 + v7 + m9) | 0; v13 ^= v2; v13 = ((v13 >>> 16) | (v13 << 16));
    v8 = (v8 + v13) | 0; v7 ^= v8; v7 = ((v7 >>> 12) | (v7 << 20));
    v2 = (v2 + v7 + m14) | 0; v13 ^= v2; v13 = ((v13 >>> 8) | (v13 << 24));
    v8 = (v8 + v13) | 0; v7 ^= v8; v7 = ((v7 >>> 7) | (v7 << 25));
    v3 = (v3 + v4 + m15) | 0; v14 ^= v3; v14 = ((v14 >>> 16) | (v14 << 16));
    v9 = (v9 + v14) | 0; v4 ^= v9; v4 = ((v4 >>> 12) | (v4 << 20));
    v3 = (v3 + v4 + m8) | 0; v14 ^= v3; v14 = ((v14 >>> 8) | (v14 << 24));
    v9 = (v9 + v14) | 0; v4 ^= v9; v4 = ((v4 >>> 7) | (v4 << 25));
    // round 3
    v0 = (v0 + v4 + m3) | 0; v12 ^= v0; v12 = ((v12 >>> 16) | (v12 << 16));
    v8 = (v8 + v12) | 0; v4 ^= v8; v4 = ((v4 >>> 12) | (v4 << 20));
    v0 = (v0 + v4 + m4) | 0; v12 ^= v0; v12 = ((v12 >>> 8) | (v12 << 24));
    v8 = (v8 + v12) | 0; v4 ^= v8; v4 = ((v4 >>> 7) | (v4 << 25));
    v1 = (v1 + v5 + m10) | 0; v13 ^= v1; v13 = ((v13 >>> 16) | (v13 << 16));
    v9 = (v9 + v13) | 0; v5 ^= v9; v5 = ((v5 >>> 12) | (v5 << 20));
    v1 = (v1 + v5 + m12) | 0; v13 ^= v1; v13 = ((v13 >>> 8) | (v13 << 24));
    v9 = (v9 + v13) | 0; v5 ^= v9; v5 = ((v5 >>> 7) | (v5 << 25));
    v2 = (v2 + v6 + m13) | 0; v14 ^= v2; v14 = ((v14 >>> 16) | (v14 << 16));
    v10 = (v10 + v14) | 0; v6 ^= v10; v6 = ((v6 >>> 12) | (v6 << 20));
    v2 = (v2 + v6 + m2) | 0; v14 ^= v2; v14 = ((v14 >>> 8) | (v14 << 24));
    v10 = (v10 + v14) | 0; v6 ^= v10; v6 = ((v6 >>> 7) | (v6 << 25));
    v3 = (v3 + v7 + m7) | 0; v15 ^= v3; v15 = ((v15 >>> 16) | (v15 << 16));
    v11 = (v11 + v15) | 0; v7 ^= v11; v7 = ((v7 >>> 12) | (v7 << 20));
    v3 = (v3 + v7 + m14) | 0; v15 ^= v3; v15 = ((v15 >>> 8) | (v15 << 24));
    v11 = (v11 + v15) | 0; v7 ^= v11; v7 = ((v7 >>> 7) | (v7 << 25));
    v0 = (v0 + v5 + m6) | 0; v15 ^= v0; v15 = ((v15 >>> 16) | (v15 << 16));
    v10 = (v10 + v15) | 0; v5 ^= v10; v5 = ((v5 >>> 12) | (v5 << 20));
    v0 = (v0 + v5 + m5) | 0; v15 ^= v0; v15 = ((v15 >>> 8) | (v15 << 24));
    v10 = (v10 + v15) | 0; v5 ^= v10; v5 = ((v5 >>> 7) | (v5 << 25));
    v1 = (v1 + v6 + m9) | 0; v12 ^= v1; v12 = ((v12 >>> 16) | (v12 << 16));
    v11 = (v11 + v12) | 0; v6 ^= v11; v6 = ((v6 >>> 12) | (v6 << 20));
    v1 = (v1 + v6 + m0) | 0; v12 ^= v1; v12 = ((v12 >>> 8) | (v12 << 24));
    v11 = (v11 + v12) | 0; v6 ^= v11; v6 = ((v6 >>> 7) | (v6 << 25));
    v2 = (v2 + v7 + m11) | 0; v13 ^= v2; v13 = ((v13 >>> 16) | (v13 << 16));
    v8 = (v8 + v13) | 0; v7 ^= v8; v7 = ((v7 >>> 12) | (v7 << 20));
    v2 = (v2 + v7 + m15) | 0; v13 ^= v2; v13 = ((v13 >>> 8) | (v13 << 24));
    v8 = (v8 + v13) | 0; v7 ^= v8; v7 = ((v7 >>> 7) | (v7 << 25));
    v3 = (v3 + v4 + m8) | 0; v14 ^= v3; v14 = ((v14 >>> 16) | (v14 << 16));
    v9 = (v9 + v14) | 0; v4 ^= v9; v4 = ((v4 >>> 12) | (v4 << 20));
    v3 = (v3 + v4 + m1) | 0; v14 ^= v3; v14 = ((v14 >>> 8) | (v14 << 24));
    v9 = (v9 + v14) | 0; v4 ^= v9; v4 = ((v4 >>> 7) | (v4 << 25));
    // round 4
    v0 = (v0 + v4 + m10) | 0; v12 ^= v0; v12 = ((v12 >>> 16) | (v12 << 16));
    v8 = (v8 + v12) | 0; v4 ^= v8; v4 = ((v4 >>> 12) | (v4 << 20));
    v0 = (v0 + v4 + m7) | 0; v12 ^= v0; v12 = ((v12 >>> 8) | (v12 << 24));
    v8 = (v8 + v12) | 0; v4 ^= v8; v4 = ((v4 >>> 7) | (v4 << 25));
    v1 = (v1 + v5 + m12) | 0; v13 ^= v1; v13 = ((v13 >>> 16) | (v13 << 16));
    v9 = (v9 + v13) | 0; v5 ^= v9; v5 = ((v5 >>> 12) | (v5 << 20));
    v1 = (v1 + v5 + m9) | 0; v13 ^= v1; v13 = ((v13 >>> 8) | (v13 << 24));
    v9 = (v9 + v13) | 0; v5 ^= v9; v5 = ((v5 >>> 7) | (v5 << 25));
    v2 = (v2 + v6 + m14) | 0; v14 ^= v2; v14 = ((v14 >>> 16) | (v14 << 16));
    v10 = (v10 + v14) | 0; v6 ^= v10; v6 = ((v6 >>> 12) | (v6 << 20));
    v2 = (v2 + v6 + m3) | 0; v14 ^= v2; v14 = ((v14 >>> 8) | (v14 << 24));
    v10 = (v10 + v14) | 0; v6 ^= v10; v6 = ((v6 >>> 7) | (v6 << 25));
    v3 = (v3 + v7 + m13) | 0; v15 ^= v3; v15 = ((v15 >>> 16) | (v15 << 16));
    v11 = (v11 + v15) | 0; v7 ^= v11; v7 = ((v7 >>> 12) | (v7 << 20));
    v3 = (v3 + v7 + m15) | 0; v15 ^= v3; v15 = ((v15 >>> 8) | (v15 << 24));
    v11 = (v11 + v15) | 0; v7 ^= v11; v7 = ((v7 >>> 7) | (v7 << 25));
    v0 = (v0 + v5 + m4) | 0; v15 ^= v0; v15 = ((v15 >>> 16) | (v15 << 16));
    v10 = (v10 + v15) | 0; v5 ^= v10; v5 = ((v5 >>> 12) | (v5 << 20));
    v0 = (v0 + v5 + m0) | 0; v15 ^= v0; v15 = ((v15 >>> 8) | (v15 << 24));
    v10 = (v10 + v15) | 0; v5 ^= v10; v5 = ((v5 >>> 7) | (v5 << 25));
    v1 = (v1 + v6 + m11) | 0; v12 ^= v1; v12 = ((v12 >>> 16) | (v12 << 16));
    v11 = (v11 + v12) | 0; v6 ^= v11; v6 = ((v6 >>> 12) | (v6 << 20));
    v1 = (v1 + v6 + m2) | 0; v12 ^= v1; v12 = ((v12 >>> 8) | (v12 << 24));
    v11 = (v11 + v12) | 0; v6 ^= v11; v6 = ((v6 >>> 7) | (v6 << 25));
    v2 = (v2 + v7 + m5) | 0; v13 ^= v2; v13 = ((v13 >>> 16) | (v13 << 16));
    v8 = (v8 + v13) | 0; v7 ^= v8; v7 = ((v7 >>> 12) | (v7 << 20));
    v2 = (v2 + v7 + m8) | 0; v13 ^= v2; v13 = ((v13 >>> 8) | (v13 << 24));
    v8 = (v8 + v13) | 0; v7 ^= v8; v7 = ((v7 >>> 7) | (v7 << 25));
    v3 = (v3 + v4 + m1) | 0; v14 ^= v3; v14 = ((v14 >>> 16) | (v14 << 16));
    v9 = (v9 + v14) | 0; v4 ^= v9; v4 = ((v4 >>> 12) | (v4 << 20));
    v3 = (v3 + v4 + m6) | 0; v14 ^= v3; v14 = ((v14 >>> 8) | (v14 << 24));
    v9 = (v9 + v14) | 0; v4 ^= v9; v4 = ((v4 >>> 7) | (v4 << 25));
    // round 5
    v0 = (v0 + v4 + m12) | 0; v12 ^= v0; v12 = ((v12 >>> 16) | (v12 << 16));
    v8 = (v8 + v12) | 0; v4 ^= v8; v4 = ((v4 >>> 12) | (v4 << 20));
    v0 = (v0 + v4 + m13) | 0; v12 ^= v0; v12 = ((v12 >>> 8) | (v12 << 24));
    v8 = (v8 + v12) | 0; v4 ^= v8; v4 = ((v4 >>> 7) | (v4 << 25));
    v1 = (v1 + v5 + m9) | 0; v13 ^= v1; v13 = ((v13 >>> 16) | (v13 << 16));
    v9 = (v9 + v13) | 0; v5 ^= v9; v5 = ((v5 >>> 12) | (v5 << 20));
    v1 = (v1 + v5 + m11) | 0; v13 ^= v1; v13 = ((v13 >>> 8) | (v13 << 24));
    v9 = (v9 + v13) | 0; v5 ^= v9; v5 = ((v5 >>> 7) | (v5 << 25));
    v2 = (v2 + v6 + m15) | 0; v14 ^= v2; v14 = ((v14 >>> 16) | (v14 << 16));
    v10 = (v10 + v14) | 0; v6 ^= v10; v6 = ((v6 >>> 12) | (v6 << 20));
    v2 = (v2 + v6 + m10) | 0; v14 ^= v2; v14 = ((v14 >>> 8) | (v14 << 24));
    v10 = (v10 + v14) | 0; v6 ^= v10; v6 = ((v6 >>> 7) | (v6 << 25));
    v3 = (v3 + v7 + m14) | 0; v15 ^= v3; v15 = ((v15 >>> 16) | (v15 << 16));
    v11 = (v11 + v15) | 0; v7 ^= v11; v7 = ((v7 >>> 12) | (v7 << 20));
    v3 = (v3 + v7 + m8) | 0; v15 ^= v3; v15 = ((v15 >>> 8) | (v15 << 24));
    v11 = (v11 + v15) | 0; v7 ^= v11; v7 = ((v7 >>> 7) | (v7 << 25));
    v0 = (v0 + v5 + m7) | 0; v15 ^= v0; v15 = ((v15 >>> 16) | (v15 << 16));
    v10 = (v10 + v15) | 0; v5 ^= v10; v5 = ((v5 >>> 12) | (v5 << 20));
    v0 = (v0 + v5 + m2) | 0; v15 ^= v0; v15 = ((v15 >>> 8) | (v15 << 24));
    v10 = (v10 + v15) | 0; v5 ^= v10; v5 = ((v5 >>> 7) | (v5 << 25));
    v1 = (v1 + v6 + m5) | 0; v12 ^= v1; v12 = ((v12 >>> 16) | (v12 << 16));
    v11 = (v11 + v12) | 0; v6 ^= v11; v6 = ((v6 >>> 12) | (v6 << 20));
    v1 = (v1 + v6 + m3) | 0; v12 ^= v1; v12 = ((v12 >>> 8) | (v12 << 24));
    v11 = (v11 + v12) | 0; v6 ^= v11; v6 = ((v6 >>> 7) | (v6 << 25));
    v2 = (v2 + v7 + m0) | 0; v13 ^= v2; v13 = ((v13 >>> 16) | (v13 << 16));
    v8 = (v8 + v13) | 0; v7 ^= v8; v7 = ((v7 >>> 12) | (v7 << 20));
    v2 = (v2 + v7 + m1) | 0; v13 ^= v2; v13 = ((v13 >>> 8) | (v13 << 24));
    v8 = (v8 + v13) | 0; v7 ^= v8; v7 = ((v7 >>> 7) | (v7 << 25));
    v3 = (v3 + v4 + m6) | 0; v14 ^= v3; v14 = ((v14 >>> 16) | (v14 << 16));
    v9 = (v9 + v14) | 0; v4 ^= v9; v4 = ((v4 >>> 12) | (v4 << 20));
    v3 = (v3 + v4 + m4) | 0; v14 ^= v3; v14 = ((v14 >>> 8) | (v14 << 24));
    v9 = (v9 + v14) | 0; v4 ^= v9; v4 = ((v4 >>> 7) | (v4 << 25));
    // round 6
    v0 = (v0 + v4 + m9) | 0; v12 ^= v0; v12 = ((v12 >>> 16) | (v12 << 16));
    v8 = (v8 + v12) | 0; v4 ^= v8; v4 = ((v4 >>> 12) | (v4 << 20));
    v0 = (v0 + v4 + m14) | 0; v12 ^= v0; v12 = ((v12 >>> 8) | (v12 << 24));
    v8 = (v8 + v12) | 0; v4 ^= v8; v4 = ((v4 >>> 7) | (v4 << 25));
    v1 = (v1 + v5 + m11) | 0; v13 ^= v1; v13 = ((v13 >>> 16) | (v13 << 16));
    v9 = (v9 + v13) | 0; v5 ^= v9; v5 = ((v5 >>> 12) | (v5 << 20));
    v1 = (v1 + v5 + m5) | 0; v13 ^= v1; v13 = ((v13 >>> 8) | (v13 << 24));
    v9 = (v9 + v13) | 0; v5 ^= v9; v5 = ((v5 >>> 7) | (v5 << 25));
    v2 = (v2 + v6 + m8) | 0; v14 ^= v2; v14 = ((v14 >>> 16) | (v14 << 16));
    v10 = (v10 + v14) | 0; v6 ^= v10; v6 = ((v6 >>> 12) | (v6 << 20));
    v2 = (v2 + v6 + m12) | 0; v14 ^= v2; v14 = ((v14 >>> 8) | (v14 << 24));
    v10 = (v10 + v14) | 0; v6 ^= v10; v6 = ((v6 >>> 7) | (v6 << 25));
    v3 = (v3 + v7 + m15) | 0; v15 ^= v3; v15 = ((v15 >>> 16) | (v15 << 16));
    v11 = (v11 + v15) | 0; v7 ^= v11; v7 = ((v7 >>> 12) | (v7 << 20));
    v3 = (v3 + v7 + m1) | 0; v15 ^= v3; v15 = ((v15 >>> 8) | (v15 << 24));
    v11 = (v11 + v15) | 0; v7 ^= v11; v7 = ((v7 >>> 7) | (v7 << 25));
    v0 = (v0 + v5 + m13) | 0; v15 ^= v0; v15 = ((v15 >>> 16) | (v15 << 16));
    v10 = (v10 + v15) | 0; v5 ^= v10; v5 = ((v5 >>> 12) | (v5 << 20));
    v0 = (v0 + v5 + m3) | 0; v15 ^= v0; v15 = ((v15 >>> 8) | (v15 << 24));
    v10 = (v10 + v15) | 0; v5 ^= v10; v5 = ((v5 >>> 7) | (v5 << 25));
    v1 = (v1 + v6 + m0) | 0; v12 ^= v1; v12 = ((v12 >>> 16) | (v12 << 16));
    v11 = (v11 + v12) | 0; v6 ^= v11; v6 = ((v6 >>> 12) | (v6 << 20));
    v1 = (v1 + v6 + m10) | 0; v12 ^= v1; v12 = ((v12 >>> 8) | (v12 << 24));
    v11 = (v11 + v12) | 0; v6 ^= v11; v6 = ((v6 >>> 7) | (v6 << 25));
    v2 = (v2 + v7 + m2) | 0; v13 ^= v2; v13 = ((v13 >>> 16) | (v13 << 16));
    v8 = (v8 + v13) | 0; v7 ^= v8; v7 = ((v7 >>> 12) | (v7 << 20));
    v2 = (v2 + v7 + m6) | 0; v13 ^= v2; v13 = ((v13 >>> 8) | (v13 << 24));
    v8 = (v8 + v13) | 0; v7 ^= v8; v7 = ((v7 >>> 7) | (v7 << 25));
    v3 = (v3 + v4 + m4) | 0; v14 ^= v3; v14 = ((v14 >>> 16) | (v14 << 16));
    v9 = (v9 + v14) | 0; v4 ^= v9; v4 = ((v4 >>> 12) | (v4 << 20));
    v3 = (v3 + v4 + m7) | 0; v14 ^= v3; v14 = ((v14 >>> 8) | (v14 << 24));
    v9 = (v9 + v14) | 0; v4 ^= v9; v4 = ((v4 >>> 7) | (v4 << 25));
    // round 7
    v0 = (v0 + v4 + m11) | 0; v12 ^= v0; v12 = ((v12 >>> 16) | (v12 << 16));
    v8 = (v8 + v12) | 0; v4 ^= v8; v4 = ((v4 >>> 12) | (v4 << 20));
    v0 = (v0 + v4 + m15) | 0; v12 ^= v0; v12 = ((v12 >>> 8) | (v12 << 24));
    v8 = (v8 + v12) | 0; v4 ^= v8; v4 = ((v4 >>> 7) | (v4 << 25));
    v1 = (v1 + v5 + m5) | 0; v13 ^= v1; v13 = ((v13 >>> 16) | (v13 << 16));
    v9 = (v9 + v13) | 0; v5 ^= v9; v5 = ((v5 >>> 12) | (v5 << 20));
    v1 = (v1 + v5 + m0) | 0; v13 ^= v1; v13 = ((v13 >>> 8) | (v13 << 24));
    v9 = (v9 + v13) | 0; v5 ^= v9; v5 = ((v5 >>> 7) | (v5 << 25));
    v2 = (v2 + v6 + m1) | 0; v14 ^= v2; v14 = ((v14 >>> 16) | (v14 << 16));
    v10 = (v10 + v14) | 0; v6 ^= v10; v6 = ((v6 >>> 12) | (v6 << 20));
    v2 = (v2 + v6 + m9) | 0; v14 ^= v2; v14 = ((v14 >>> 8) | (v14 << 24));
    v10 = (v10 + v14) | 0; v6 ^= v10; v6 = ((v6 >>> 7) | (v6 << 25));
    v3 = (v3 + v7 + m8) | 0; v15 ^= v3; v15 = ((v15 >>> 16) | (v15 << 16));
    v11 = (v11 + v15) | 0; v7 ^= v11; v7 = ((v7 >>> 12) | (v7 << 20));
    v3 = (v3 + v7 + m6) | 0; v15 ^= v3; v15 = ((v15 >>> 8) | (v15 << 24));
    v11 = (v11 + v15) | 0; v7 ^= v11; v7 = ((v7 >>> 7) | (v7 << 25));
    v0 = (v0 + v5 + m14) | 0; v15 ^= v0; v15 = ((v15 >>> 16) | (v15 << 16));
    v10 = (v10 + v15) | 0; v5 ^= v10; v5 = ((v5 >>> 12) | (v5 << 20));
    v0 = (v0 + v5 + m10) | 0; v15 ^= v0; v15 = ((v15 >>> 8) | (v15 << 24));
    v10 = (v10 + v15) | 0; v5 ^= v10; v5 = ((v5 >>> 7) | (v5 << 25));
    v1 = (v1 + v6 + m2) | 0; v12 ^= v1; v12 = ((v12 >>> 16) | (v12 << 16));
    v11 = (v11 + v12) | 0; v6 ^= v11; v6 = ((v6 >>> 12) | (v6 << 20));
    v1 = (v1 + v6 + m12) | 0; v12 ^= v1; v12 = ((v12 >>> 8) | (v12 << 24));
    v11 = (v11 + v12) | 0; v6 ^= v11; v6 = ((v6 >>> 7) | (v6 << 25));
    v2 = (v2 + v7 + m3) | 0; v13 ^= v2; v13 = ((v13 >>> 16) | (v13 << 16));
    v8 = (v8 + v13) | 0; v7 ^= v8; v7 = ((v7 >>> 12) | (v7 << 20));
    v2 = (v2 + v7 + m4) | 0; v13 ^= v2; v13 = ((v13 >>> 8) | (v13 << 24));
    v8 = (v8 + v13) | 0; v7 ^= v8; v7 = ((v7 >>> 7) | (v7 << 25));
    v3 = (v3 + v4 + m7) | 0; v14 ^= v3; v14 = ((v14 >>> 16) | (v14 << 16));
    v9 = (v9 + v14) | 0; v4 ^= v9; v4 = ((v4 >>> 12) | (v4 << 20));
    v3 = (v3 + v4 + m13) | 0; v14 ^= v3; v14 = ((v14 >>> 8) | (v14 << 24));
    v9 = (v9 + v14) | 0; v4 ^= v9; v4 = ((v4 >>> 7) | (v4 << 25));

  out[oo] = v0 ^ v8; out[oo + 1] = v1 ^ v9; out[oo + 2] = v2 ^ v10; out[oo + 3] = v3 ^ v11;
  out[oo + 4] = v4 ^ v12; out[oo + 5] = v5 ^ v13; out[oo + 6] = v6 ^ v14; out[oo + 7] = v7 ^ v15;
  if (full) {
    out[oo + 8] = v8 ^ c0; out[oo + 9] = v9 ^ c1; out[oo + 10] = v10 ^ c2; out[oo + 11] = v11 ^ c3;
    out[oo + 12] = v12 ^ c4; out[oo + 13] = v13 ^ c5; out[oo + 14] = v14 ^ c6; out[oo + 15] = v15 ^ c7;
  }
}

/** Little-endian words from `bytes` (any alignment, any platform). */
function readWords(bytes: Uint8Array, offset: number, length: number, words: Uint32Array): void {
  words.fill(0);
  for (let i = 0; i < length; i++) words[i >>> 2]! |= bytes[offset + i]! << ((i & 3) * 8);
}

/** A digest as 64 lowercase hex digits. */
export function toHex(digest: Uint8Array): string {
  let text = '';
  for (const byte of digest) text += byte.toString(16).padStart(2, '0');
  return text;
}

/** Incremental BLAKE3 over a stream of bytes. */
export class Blake3 {
  /** Complete subtrees awaiting their right sibling, 8 words each. */
  private readonly stack = new Uint32Array(64 * 8);
  private stackLength = 0;
  /** The chunk being filled. */
  private readonly chunkCv = new Uint32Array(8);
  private chunkCounter: number;
  private readonly block = new Uint8Array(BLOCK_LEN);
  private readonly blockWords = new Uint32Array(16);
  private blockLength = 0;
  private blocksCompressed = 0;
  private readonly scratch = new Uint32Array(16);
  private readonly parentBlock = new Uint32Array(16);
  private bytes = 0;

  /** `firstChunk` starts the hasher inside a larger input (subtree hashing):
   *  it must be a multiple of the subtree size the caller will finish. */
  constructor(firstChunk = 0) {
    if (!Number.isSafeInteger(firstChunk) || firstChunk < 0) throw new Error('The first chunk index is invalid.');
    this.chunkCounter = firstChunk;
    this.chunkCv.set(IV);
  }

  /** Bytes fed to this hasher (not counting pushed subtrees). */
  get length(): number {
    return this.bytes;
  }

  update(input: Uint8Array): this {
    const length = input.length;
    this.bytes += length;
    const words = LITTLE_ENDIAN && (input.byteOffset & 3) === 0
      ? new Uint32Array(input.buffer, input.byteOffset, length >>> 2) : null;
    let position = 0;
    while (position < length) {
      if (this.blockLength === BLOCK_LEN && this.blocksCompressed === 15) {
        // A full chunk with more input after it: it is not the last chunk.
        this.finishChunk();
      }
      // Whole chunks straight from the input when nothing is buffered. The
      // last chunk of the input stays buffered: it may be the root.
      if (words && this.blockLength === 0 && this.blocksCompressed === 0 && (position & 3) === 0 && length - position > CHUNK_LEN) {
        const cv = this.chunkCv;
        let wordOffset = position >>> 2;
        for (let block = 0; block < 16; block++, wordOffset += 16) {
          const flags = (block === 0 ? CHUNK_START : 0) | (block === 15 ? CHUNK_END : 0);
          compress(cv, 0, words, wordOffset, this.chunkCounter, BLOCK_LEN, flags, cv, 0, false);
        }
        this.pushChunk();
        position += CHUNK_LEN;
        continue;
      }
      if (this.blockLength === BLOCK_LEN) {
        readWords(this.block, 0, BLOCK_LEN, this.blockWords);
        compress(this.chunkCv, 0, this.blockWords, 0, this.chunkCounter, BLOCK_LEN, this.blocksCompressed === 0 ? CHUNK_START : 0, this.chunkCv, 0, false);
        this.blocksCompressed++;
        this.blockLength = 0;
      }
      const take = Math.min(BLOCK_LEN - this.blockLength, length - position);
      this.block.set(input.subarray(position, position + take), this.blockLength);
      this.blockLength += take;
      position += take;
    }
    return this;
  }

  /**
   * Adds the chaining value of a complete subtree of `chunks` (a power of
   * two) chunks that starts where this hasher is. Only valid while nothing
   * is buffered and the position is a multiple of `chunks`.
   */
  pushSubtree(chainingValue: Uint32Array, chunks: number): this {
    if (chainingValue.length !== 8 || !Number.isSafeInteger(chunks) || chunks < 1 || !Number.isInteger(Math.log2(chunks))) {
      throw new Error('A subtree is 8 words over a power-of-two number of chunks.');
    }
    if (this.blockLength !== 0 || this.blocksCompressed !== 0 || this.chunkCounter % chunks !== 0) {
      throw new Error('A subtree must start on its own boundary with nothing buffered.');
    }
    const cv = this.scratch;
    cv.set(chainingValue);
    this.chunkCounter += chunks;
    this.bytes += chunks * CHUNK_LEN;
    this.mergeAndPush(cv, this.chunkCounter / chunks);
    return this;
  }

  /** The 32-byte digest of everything so far. The hasher can keep going. */
  digest(): Uint8Array {
    // The current chunk's output, then parents up the stack, the last as root.
    const blockWords = new Uint32Array(16);
    readWords(this.block, 0, this.blockLength, blockWords);
    const inputCv = new Uint32Array(this.chunkCv);
    let counter = this.chunkCounter;
    let blockLength = this.blockLength;
    let flags = CHUNK_END | (this.blocksCompressed === 0 ? CHUNK_START : 0);
    for (let index = this.stackLength - 1; index >= 0; index--) {
      const cv = new Uint32Array(8);
      compress(inputCv, 0, blockWords, 0, counter, blockLength, flags, cv, 0, false);
      blockWords.set(this.stack.subarray(index * 8, index * 8 + 8), 0);
      blockWords.set(cv, 8);
      inputCv.set(IV);
      counter = 0;
      blockLength = BLOCK_LEN;
      flags = PARENT;
    }
    const out = new Uint32Array(16);
    compress(inputCv, 0, blockWords, 0, 0, blockLength, flags | ROOT, out, 0, false);
    const digest = new Uint8Array(32);
    for (let i = 0; i < 8; i++) {
      const word = out[i]!;
      digest[i * 4] = word & 0xff; digest[i * 4 + 1] = (word >>> 8) & 0xff;
      digest[i * 4 + 2] = (word >>> 16) & 0xff; digest[i * 4 + 3] = word >>> 24;
    }
    return digest;
  }

  hex(): string {
    return toHex(this.digest());
  }

  /**
   * The chaining value of the subtree this hasher was fed: a power-of-two
   * number of whole chunks starting at its first chunk. Never a root.
   */
  subtreeChainingValue(): Uint32Array {
    if (this.blockLength !== BLOCK_LEN || this.blocksCompressed !== 15) throw new Error('A subtree ends on a whole chunk.');
    const cv = new Uint32Array(8);
    readWords(this.block, 0, BLOCK_LEN, this.blockWords);
    compress(this.chunkCv, 0, this.blockWords, 0, this.chunkCounter, BLOCK_LEN, CHUNK_END, cv, 0, false);
    for (let index = this.stackLength - 1; index >= 0; index--) this.parent(this.stack, index * 8, cv, cv);
    return cv;
  }

  private finishChunk(): void {
    readWords(this.block, 0, BLOCK_LEN, this.blockWords);
    compress(this.chunkCv, 0, this.blockWords, 0, this.chunkCounter, BLOCK_LEN, CHUNK_END, this.chunkCv, 0, false);
    this.blockLength = 0;
    this.pushChunk();
  }

  /** The chunk in `chunkCv` is complete: push it and start the next one. */
  private pushChunk(): void {
    this.chunkCounter++;
    this.scratch.set(this.chunkCv);
    this.mergeAndPush(this.scratch, this.chunkCounter);
    this.chunkCv.set(IV);
    this.blocksCompressed = 0;
    this.blockLength = 0;
  }

  /** Merges `cv` with completed left siblings while `total` is even. */
  private mergeAndPush(cv: Uint32Array, total: number): void {
    while (total % 2 === 0) {
      this.stackLength--;
      this.parent(this.stack, this.stackLength * 8, cv, cv);
      total /= 2;
    }
    this.stack.set(cv.subarray(0, 8), this.stackLength * 8);
    this.stackLength++;
  }

  /** `out` = the non-root parent of `left[lo..lo+8]` and `right`. */
  private parent(left: Uint32Array, lo: number, right: Uint32Array, out: Uint32Array): void {
    const block = this.parentBlock;
    block.set(left.subarray(lo, lo + 8), 0);
    block.set(right.subarray(0, 8), 8);
    compress(IV, 0, block, 0, 0, BLOCK_LEN, PARENT, out, 0, false);
  }
}

/** The BLAKE3 digest of `bytes` as hex. */
export function blake3Hex(bytes: Uint8Array): string {
  return new Blake3().update(bytes).hex();
}
