/*
 * The canonical binary patch format, version 1: the body of every engine
 * PatchBatch (contract 5, PatchEncoding `canonicalV1`; normative text in
 * docs/engine/rfcs/05-patches.md). The renderer encodes here; the engine
 * decodes in engine/crates/pm-model (src/binary.rs). Golden vectors generated
 * from this module (engine/crates/pm-model/tests/vectors/
 * patch-binary-vectors.json) pin both sides to the same bytes.
 *
 *   body   = magic count op*             magic = 50 4D 50 01 ("PMP", version 1)
 *   count  = u32                         number of operations
 *   op     = 0x00 path                   leaf delete     { path, exists: false }
 *          | 0x01 path json              leaf set        { path, exists: true, value }
 *          | 0x02 path                   leaf set        { path, exists: true } (value undefined)
 *          | 0x10 path u32 str*          order           { op: 'order', path, ids }
 *          | 0x11 path u32 json          insert          { op: 'insert', path, index, value }
 *          | 0x12 path str               delete by id    { op: 'delete', path, id }
 *   path   = u32 part*                   number of parts, then the parts
 *   part   = 0x00 str                    string key
 *          | 0x01 u32                    integer index 0 to 2^32 - 1
 *          | 0x02 f64                    any other non-negative integer (-0, or 2^32 and above)
 *   str    = u32 bytes                   WTF-8 (UTF-8, plus unpaired surrogates as 3 bytes)
 *   json   = u32 bytes                   UTF-8 JSON text
 *
 * Integers are little-endian and u32 lengths count bytes. Path numbers are
 * non-negative integers, as isPatchList requires. The encoder writes JSON as
 * canonicalStringify (sorted keys) and always picks the shortest part form, so
 * equal operation lists encode to equal bytes. The decoder rejects every
 * structural deviation but accepts any valid JSON text.
 *
 * Structural edits must reach this encoder as id-keyed operations
 * (patch-ids.ts diffOps), never as a stringified whole array: S5 measured the
 * whole-array stringify alone over the 2 ms structural budget.
 */
import { canonicalStringify } from './canonical-json';
import type { PathPart } from './patch';
import type { PatchOp } from './patch-ids';

export const PATCH_BINARY_MAGIC: readonly number[] = [0x50, 0x4d, 0x50, 0x01];

export const OP_TAG = { leafDelete: 0x00, leafSet: 0x01, leafSetUndefined: 0x02, order: 0x10, insert: 0x11, deleteId: 0x12 } as const;
export const PART_TAG = { key: 0x00, index: 0x01, number: 0x02 } as const;

/** Thrown by encodePatchOps for operations the format cannot carry and by decodePatchOps for malformed bodies. */
export class PatchBinaryError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'PatchBinaryError';
  }
}

const SURROGATE = /[\ud800-\udfff]/;
const utf8 = new TextEncoder();

class Writer {
  bytes = new Uint8Array(256);
  view = new DataView(this.bytes.buffer);
  length = 0;

  reserve(n: number) {
    if (this.length + n <= this.bytes.length) return;
    let size = this.bytes.length * 2;
    while (size < this.length + n) size *= 2;
    const next = new Uint8Array(size);
    next.set(this.bytes.subarray(0, this.length));
    this.bytes = next;
    this.view = new DataView(next.buffer);
  }

  u8(n: number) {
    this.reserve(1);
    this.bytes[this.length++] = n;
  }

  u32(n: number) {
    if (!Number.isInteger(n) || n < 0 || n > 0xffffffff) throw new PatchBinaryError(`${n} does not fit in 32 bits`);
    this.reserve(4);
    this.view.setUint32(this.length, n, true);
    this.length += 4;
  }

  f64(n: number) {
    this.reserve(8);
    this.view.setFloat64(this.length, n, true);
    this.length += 8;
  }

  /** A length-prefixed text: UTF-8 when well-formed, WTF-8 when it holds lone surrogates. */
  text(s: string) {
    if (SURROGATE.test(s)) {
      const bytes = wtf8(s);
      this.u32(bytes.length);
      this.reserve(bytes.length);
      this.bytes.set(bytes, this.length);
      this.length += bytes.length;
      return;
    }
    this.reserve(4 + s.length * 3);
    const { written } = utf8.encodeInto(s, this.bytes.subarray(this.length + 4));
    this.view.setUint32(this.length, written, true);
    this.length += 4 + written;
  }

  path(path: readonly PathPart[]) {
    if (!Array.isArray(path)) throw new PatchBinaryError('an operation needs a path array');
    this.u32(path.length);
    for (const part of path) {
      if (typeof part === 'string') {
        this.u8(PART_TAG.key);
        this.text(part);
      } else if (typeof part === 'number' && Number.isInteger(part) && part >= 0) {
        if (Object.is(part, -0) || part > 0xffffffff) {
          this.u8(PART_TAG.number);
          this.f64(part);
        } else {
          this.u8(PART_TAG.index);
          this.u32(part);
        }
      } else {
        throw new PatchBinaryError('path parts must be strings or non-negative integers');
      }
    }
  }

  json(value: unknown) {
    const text = canonicalStringify(value);
    if (text === undefined) throw new PatchBinaryError('the value has no JSON representation');
    this.text(text);
  }
}

/** WTF-8: UTF-8 with paired surrogates combined and lone ones as 3-byte sequences. */
function wtf8(s: string): Uint8Array {
  const out: number[] = [];
  for (let i = 0; i < s.length; i++) {
    let cp = s.charCodeAt(i);
    if (cp >= 0xd800 && cp <= 0xdbff && i + 1 < s.length) {
      const low = s.charCodeAt(i + 1);
      if (low >= 0xdc00 && low <= 0xdfff) {
        cp = 0x10000 + ((cp - 0xd800) << 10) + (low - 0xdc00);
        i++;
      }
    }
    if (cp < 0x80) out.push(cp);
    else if (cp < 0x800) out.push(0xc0 | (cp >> 6), 0x80 | (cp & 0x3f));
    else if (cp < 0x10000) out.push(0xe0 | (cp >> 12), 0x80 | ((cp >> 6) & 0x3f), 0x80 | (cp & 0x3f));
    else out.push(0xf0 | (cp >> 18), 0x80 | ((cp >> 12) & 0x3f), 0x80 | ((cp >> 6) & 0x3f), 0x80 | (cp & 0x3f));
  }
  return Uint8Array.from(out);
}

const recordId = (value: any): string | undefined =>
  value && typeof value === 'object' && !Array.isArray(value) && typeof value.id === 'string' ? value.id : undefined;

/** Encodes leaf patches and id-keyed operations in the canonical form. */
export function encodePatchOps(ops: readonly PatchOp[]): Uint8Array {
  const w = new Writer();
  for (const byte of PATCH_BINARY_MAGIC) w.u8(byte);
  w.u32(ops.length);
  ops.forEach((op, index) => {
    try {
      if ('op' in op && typeof op.op === 'string') {
        switch (op.op) {
          case 'order':
            w.u8(OP_TAG.order);
            w.path(op.path);
            w.u32(op.ids.length);
            for (const id of op.ids) {
              if (typeof id !== 'string') throw new PatchBinaryError('order ids must be strings');
              w.text(id);
            }
            return;
          case 'insert':
            if (recordId(op.value) === undefined) throw new PatchBinaryError('an inserted value must be an object with a string id');
            w.u8(OP_TAG.insert);
            w.path(op.path);
            w.u32(op.index);
            w.json(op.value);
            return;
          case 'delete':
            if (typeof op.id !== 'string') throw new PatchBinaryError('delete needs a string id');
            w.u8(OP_TAG.deleteId);
            w.path(op.path);
            w.text(op.id);
            return;
          default:
            throw new PatchBinaryError(`unknown operation '${String((op as { op: unknown }).op)}'`);
        }
      }
      if (!op.exists) {
        w.u8(OP_TAG.leafDelete);
        w.path(op.path);
      } else if (op.value === undefined) {
        w.u8(OP_TAG.leafSetUndefined);
        w.path(op.path);
      } else {
        w.u8(OP_TAG.leafSet);
        w.path(op.path);
        w.json(op.value);
      }
    } catch (error) {
      if (error instanceof PatchBinaryError) throw new PatchBinaryError(`operation ${index}: ${error.message}`);
      throw error;
    }
  });
  return w.bytes.slice(0, w.length);
}

// ── Decoding ──────────────────────────────────────────────────────────────

const strictUtf8 = new TextDecoder('utf-8', { fatal: true, ignoreBOM: true });

/** Reads WTF-8, rejecting what pm-model's JsString::from_wtf8 rejects. */
function readWtf8(bytes: Uint8Array): string {
  try {
    return strictUtf8.decode(bytes);
  } catch {
    // Not UTF-8: lone surrogates, or invalid.
  }
  const units: number[] = [];
  const cont = (i: number) => {
    const b = bytes[i];
    if (b === undefined || (b & 0xc0) !== 0x80) throw new PatchBinaryError('invalid WTF-8 string');
    return b & 0x3f;
  };
  let previousHigh = false;
  for (let i = 0; i < bytes.length;) {
    const b0 = bytes[i]!;
    let cp: number;
    if (b0 < 0x80) { cp = b0; i += 1; }
    else if (b0 >= 0xc2 && b0 <= 0xdf) { cp = ((b0 & 0x1f) << 6) | cont(i + 1); i += 2; }
    else if (b0 >= 0xe0 && b0 <= 0xef) {
      if (b0 === 0xe0 && (bytes[i + 1] ?? 0) < 0xa0) throw new PatchBinaryError('invalid WTF-8 string');
      cp = ((b0 & 0x0f) << 12) | (cont(i + 1) << 6) | cont(i + 2);
      i += 3;
    } else if (b0 >= 0xf0 && b0 <= 0xf4) {
      const b1 = bytes[i + 1] ?? 0;
      if ((b0 === 0xf0 && b1 < 0x90) || (b0 === 0xf4 && b1 > 0x8f)) throw new PatchBinaryError('invalid WTF-8 string');
      cp = ((b0 & 0x07) << 18) | (cont(i + 1) << 12) | (cont(i + 2) << 6) | cont(i + 3);
      i += 4;
    } else throw new PatchBinaryError('invalid WTF-8 string');
    if (previousHigh && cp >= 0xdc00 && cp <= 0xdfff) throw new PatchBinaryError('invalid WTF-8 string');
    previousHigh = cp >= 0xd800 && cp <= 0xdbff;
    if (cp >= 0x10000) units.push(0xd800 + ((cp - 0x10000) >> 10), 0xdc00 + ((cp - 0x10000) & 0x3ff));
    else units.push(cp);
  }
  let out = '';
  for (let i = 0; i < units.length; i += 4096) out += String.fromCharCode(...units.slice(i, i + 4096));
  return out;
}

class Reader {
  at = 0;
  readonly view: DataView;
  constructor(readonly bytes: Uint8Array) {
    this.view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  }

  fail(message: string, at = this.at): never {
    throw new PatchBinaryError(`${message} at byte ${at}`);
  }

  take(n: number): Uint8Array {
    if (this.at + n > this.bytes.length) this.fail('truncated body');
    const out = this.bytes.subarray(this.at, this.at + n);
    this.at += n;
    return out;
  }

  u8(): number {
    if (this.at + 1 > this.bytes.length) this.fail('truncated body');
    return this.bytes[this.at++]!;
  }

  u32(): number {
    if (this.at + 4 > this.bytes.length) this.fail('truncated body');
    const n = this.view.getUint32(this.at, true);
    this.at += 4;
    return n;
  }

  f64(): number {
    if (this.at + 8 > this.bytes.length) this.fail('truncated body');
    const n = this.view.getFloat64(this.at, true);
    this.at += 8;
    return n;
  }

  count(min: number): number {
    const n = this.u32();
    if (n * min > this.bytes.length - this.at) this.fail('count exceeds the remaining bytes');
    return n;
  }

  text(): string {
    const at = this.at;
    const bytes = this.take(this.u32());
    try {
      return readWtf8(bytes);
    } catch {
      return this.fail('invalid WTF-8 string', at);
    }
  }

  json(): any {
    const at = this.at;
    const bytes = this.take(this.u32());
    try {
      return JSON.parse(strictUtf8.decode(bytes));
    } catch {
      return this.fail('invalid JSON value', at);
    }
  }

  path(): PathPart[] {
    const n = this.count(5);
    const path: PathPart[] = [];
    for (let i = 0; i < n; i++) {
      const at = this.at;
      const tag = this.u8();
      if (tag === PART_TAG.key) path.push(this.text());
      else if (tag === PART_TAG.index) path.push(this.u32());
      else if (tag === PART_TAG.number) {
        const n = this.f64();
        if (!(Number.isInteger(n) && n >= 0)) this.fail('path numbers must be non-negative integers', at);
        if (!Object.is(n, -0) && n <= 0xffffffff) this.fail('non-canonical path number (fits the index form)', at);
        path.push(n);
      } else this.fail('unknown path part tag', at);
    }
    return path;
  }
}

/** Decodes a version 1 body into leaf patches and id-keyed operations. */
export function decodePatchOps(bytes: Uint8Array): PatchOp[] {
  const r = new Reader(bytes);
  if (bytes.length < 4 || PATCH_BINARY_MAGIC.some((byte, index) => bytes[index] !== byte)) r.fail('not a version 1 patch body', 0);
  r.at = 4;
  const count = r.count(5);
  const ops: PatchOp[] = [];
  for (let i = 0; i < count; i++) {
    const at = r.at;
    const tag = r.u8();
    switch (tag) {
      case OP_TAG.leafDelete: ops.push({ path: r.path(), exists: false }); break;
      case OP_TAG.leafSet: { const path = r.path(); ops.push({ path, exists: true, value: r.json() }); break; }
      case OP_TAG.leafSetUndefined: ops.push({ path: r.path(), exists: true }); break;
      case OP_TAG.order: {
        const path = r.path();
        const n = r.count(4);
        const ids: string[] = [];
        for (let k = 0; k < n; k++) ids.push(r.text());
        ops.push({ op: 'order', path, ids });
        break;
      }
      case OP_TAG.insert: {
        const path = r.path();
        const index = r.u32();
        const valueAt = r.at;
        const value = r.json();
        if (recordId(value) === undefined) r.fail('an inserted value must be an object with a string id', valueAt);
        ops.push({ op: 'insert', path, index, value });
        break;
      }
      case OP_TAG.deleteId: { const path = r.path(); ops.push({ op: 'delete', path, id: r.text() }); break; }
      default: r.fail('unknown operation tag', at);
    }
  }
  if (r.at !== bytes.length) r.fail('trailing bytes after the last operation');
  return ops;
}

/** Whether `bytes` decodes and re-encodes to itself. */
export function isCanonicalPatchBody(bytes: Uint8Array): boolean {
  let again: Uint8Array;
  try {
    again = encodePatchOps(decodePatchOps(bytes));
  } catch {
    return false;
  }
  return again.length === bytes.length && again.every((byte, index) => byte === bytes[index]);
}
