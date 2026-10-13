/*
 * MessagePack for the engine protocol (contract 9,
 * docs/engine/rfcs/09-protocol-codec.md), as main reads and writes it.
 *
 * The decoder is strict because engine bytes are untrusted input to main:
 * maps have unique string keys, every integer is a safe JavaScript number,
 * every float is finite, nesting stops at 64, the input is at most 16 MiB,
 * and nothing may follow the value. `bin` values become `Uint8Array`s.
 *
 * The encoder writes what `rmp_serde::to_vec_named` writes for the same
 * value: integers in their smallest form, other numbers as float64 (Rust
 * float fields accept integers, see pm-protocol's codec tests), strings as
 * UTF-8, `Uint8Array` as `bin`, `null` and `undefined` as `nil`. Object
 * fields that are `undefined` are skipped, as serde skips nothing but the
 * generated types never use `undefined` for a present field.
 */

export const MAX_MESSAGE_BYTES = 16 * 1024 * 1024;
export const MAX_DEPTH = 64;

export class MessagePackError extends Error {
  override name = 'MessagePackError';
}

const textDecoder = new TextDecoder('utf-8', { fatal: true });
const textEncoder = new TextEncoder();

export function decode(bytes: Uint8Array): unknown {
  if (bytes.byteLength > MAX_MESSAGE_BYTES) throw new MessagePackError(`message of ${bytes.byteLength} bytes exceeds ${MAX_MESSAGE_BYTES}`);
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  let at = 0;
  const take = (n: number): number => {
    if (at + n > bytes.byteLength) throw new MessagePackError(`truncated at byte ${at}`);
    const start = at;
    at += n;
    return start;
  };
  const safe = (value: bigint): number => {
    if (value > BigInt(Number.MAX_SAFE_INTEGER) || value < BigInt(-Number.MAX_SAFE_INTEGER)) {
      throw new MessagePackError(`integer ${value} is outside the safe range`);
    }
    return Number(value);
  };
  const finite = (value: number): number => {
    if (!Number.isFinite(value)) throw new MessagePackError('non-finite float');
    return value;
  };
  const str = (n: number): string => {
    const start = take(n);
    try {
      return textDecoder.decode(bytes.subarray(start, start + n));
    } catch {
      throw new MessagePackError(`invalid UTF-8 at byte ${start}`);
    }
  };
  const bin = (n: number): Uint8Array => {
    const start = take(n);
    return bytes.slice(start, start + n);
  };
  const array = (n: number, depth: number): unknown[] => {
    if (n > bytes.byteLength - at) throw new MessagePackError('array longer than the input');
    const out: unknown[] = new Array(n);
    for (let i = 0; i < n; i++) out[i] = value(depth);
    return out;
  };
  const map = (n: number, depth: number): Record<string, unknown> => {
    if (n * 2 > bytes.byteLength - at) throw new MessagePackError('map longer than the input');
    const out: Record<string, unknown> = Object.create(null) as Record<string, unknown>;
    for (let i = 0; i < n; i++) {
      const key = value(depth);
      if (typeof key !== 'string') throw new MessagePackError('map keys must be strings');
      if (Object.prototype.hasOwnProperty.call(out, key)) throw new MessagePackError(`duplicate key ${JSON.stringify(key)}`);
      out[key] = value(depth);
    }
    return out;
  };
  function value(depth: number): unknown {
    if (depth > MAX_DEPTH) throw new MessagePackError(`nesting deeper than ${MAX_DEPTH}`);
    const byte = view.getUint8(take(1));
    if (byte <= 0x7f) return byte;
    if (byte >= 0xe0) return byte - 0x100;
    if ((byte & 0xe0) === 0xa0) return str(byte & 0x1f);
    if ((byte & 0xf0) === 0x90) return array(byte & 0x0f, depth + 1);
    if ((byte & 0xf0) === 0x80) return map(byte & 0x0f, depth + 1);
    switch (byte) {
      case 0xc0: return null;
      case 0xc2: return false;
      case 0xc3: return true;
      case 0xc4: return bin(view.getUint8(take(1)));
      case 0xc5: return bin(view.getUint16(take(2)));
      case 0xc6: return bin(view.getUint32(take(4)));
      case 0xca: return finite(view.getFloat32(take(4)));
      case 0xcb: return finite(view.getFloat64(take(8)));
      case 0xcc: return view.getUint8(take(1));
      case 0xcd: return view.getUint16(take(2));
      case 0xce: return view.getUint32(take(4));
      case 0xcf: return safe(view.getBigUint64(take(8)));
      case 0xd0: return view.getInt8(take(1));
      case 0xd1: return view.getInt16(take(2));
      case 0xd2: return view.getInt32(take(4));
      case 0xd3: return safe(view.getBigInt64(take(8)));
      case 0xd9: return str(view.getUint8(take(1)));
      case 0xda: return str(view.getUint16(take(2)));
      case 0xdb: return str(view.getUint32(take(4)));
      case 0xdc: return array(view.getUint16(take(2)), depth + 1);
      case 0xdd: return array(view.getUint32(take(4)), depth + 1);
      case 0xde: return map(view.getUint16(take(2)), depth + 1);
      case 0xdf: return map(view.getUint32(take(4)), depth + 1);
      default: throw new MessagePackError(`unsupported MessagePack byte 0x${byte.toString(16)} at ${at - 1}`);
    }
  }
  const result = value(0);
  if (at !== bytes.byteLength) throw new MessagePackError(`${bytes.byteLength - at} trailing bytes`);
  return result;
}

class Writer {
  private buffer = new Uint8Array(256);
  private view = new DataView(this.buffer.buffer);
  length = 0;

  private reserve(n: number): void {
    if (this.length + n <= this.buffer.byteLength) return;
    let size = this.buffer.byteLength * 2;
    while (size < this.length + n) size *= 2;
    if (size > MAX_MESSAGE_BYTES * 2) size = Math.max(this.length + n, MAX_MESSAGE_BYTES + 1);
    const next = new Uint8Array(size);
    next.set(this.buffer.subarray(0, this.length));
    this.buffer = next;
    this.view = new DataView(next.buffer);
  }

  u8(value: number): void { this.reserve(1); this.view.setUint8(this.length, value); this.length += 1; }
  u16(value: number): void { this.reserve(2); this.view.setUint16(this.length, value); this.length += 2; }
  u32(value: number): void { this.reserve(4); this.view.setUint32(this.length, value); this.length += 4; }
  u64(value: bigint): void { this.reserve(8); this.view.setBigUint64(this.length, value); this.length += 8; }
  i64(value: bigint): void { this.reserve(8); this.view.setBigInt64(this.length, value); this.length += 8; }
  f64(value: number): void { this.reserve(8); this.view.setFloat64(this.length, value); this.length += 8; }
  bytes(value: Uint8Array): void { this.reserve(value.byteLength); this.buffer.set(value, this.length); this.length += value.byteLength; }

  result(): Uint8Array {
    return this.buffer.slice(0, this.length);
  }
}

function writeInt(w: Writer, n: number): void {
  if (n >= 0) {
    if (n <= 0x7f) w.u8(n);
    else if (n <= 0xff) { w.u8(0xcc); w.u8(n); }
    else if (n <= 0xffff) { w.u8(0xcd); w.u16(n); }
    else if (n <= 0xffffffff) { w.u8(0xce); w.u32(n); }
    else { w.u8(0xcf); w.u64(BigInt(n)); }
  } else if (n >= -32) w.u8(n + 0x100);
  else if (n >= -0x80) { w.u8(0xd0); w.u8(n + 0x100); }
  else if (n >= -0x8000) { w.u8(0xd1); w.u16(n + 0x10000); }
  else if (n >= -0x80000000) { w.u8(0xd2); w.u32(n + 0x100000000); }
  else { w.u8(0xd3); w.i64(BigInt(n)); }
}

function writeLength(w: Writer, n: number, fix: number, fixMax: number, m8: number | null, m16: number, m32: number): void {
  if (n <= fixMax) w.u8(fix | n);
  else if (m8 !== null && n <= 0xff) { w.u8(m8); w.u8(n); }
  else if (n <= 0xffff) { w.u8(m16); w.u16(n); }
  else { w.u8(m32); w.u32(n); }
}

function write(w: Writer, value: unknown, depth: number): void {
  if (depth > MAX_DEPTH) throw new MessagePackError(`nesting deeper than ${MAX_DEPTH}`);
  if (value === null || value === undefined) { w.u8(0xc0); return; }
  switch (typeof value) {
    case 'boolean': w.u8(value ? 0xc3 : 0xc2); return;
    case 'number':
      if (!Number.isFinite(value)) throw new MessagePackError('non-finite number');
      if (Number.isInteger(value)) {
        if (!Number.isSafeInteger(value)) throw new MessagePackError(`integer ${value} is outside the safe range`);
        writeInt(w, value);
      } else {
        w.u8(0xcb);
        w.f64(value);
      }
      return;
    case 'string': {
      const utf8 = textEncoder.encode(value);
      writeLength(w, utf8.byteLength, 0xa0, 31, 0xd9, 0xda, 0xdb);
      w.bytes(utf8);
      return;
    }
    case 'object': {
      if (value instanceof Uint8Array) {
        if (value.byteLength <= 0xff) { w.u8(0xc4); w.u8(value.byteLength); }
        else if (value.byteLength <= 0xffff) { w.u8(0xc5); w.u16(value.byteLength); }
        else { w.u8(0xc6); w.u32(value.byteLength); }
        w.bytes(value);
        return;
      }
      if (Array.isArray(value)) {
        writeLength(w, value.length, 0x90, 15, null, 0xdc, 0xdd);
        for (const item of value) write(w, item, depth + 1);
        return;
      }
      const entries = Object.entries(value as Record<string, unknown>).filter(([, v]) => v !== undefined);
      writeLength(w, entries.length, 0x80, 15, null, 0xde, 0xdf);
      for (const [key, item] of entries) {
        write(w, key, depth + 1);
        write(w, item, depth + 1);
      }
      return;
    }
    default:
      throw new MessagePackError(`cannot encode a ${typeof value}`);
  }
}

export function encode(value: unknown): Uint8Array {
  const w = new Writer();
  write(w, value, 0);
  if (w.length > MAX_MESSAGE_BYTES) throw new MessagePackError(`message of ${w.length} bytes exceeds ${MAX_MESSAGE_BYTES}`);
  return w.result();
}
