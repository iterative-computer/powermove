/*
 * JSON text in bounded slices, and a content digest of JSON text.
 *
 * `serializeInSlices(value, emit)` emits exactly `JSON.stringify(value)` in
 * order, in pieces, yielding to the event loop between slices. Containers near
 * the root, and any large array or object, are walked here; every other value
 * is serialized by one native `JSON.stringify` call, so a slice costs a few
 * milliseconds even for documents of hundreds of megabytes. It follows the
 * specification's SerializeJSONProperty: `toJSON(key)` with the member's own
 * key, unwrapped primitive wrappers, `undefined`/functions/symbols omitted from
 * objects and written as `null` in arrays, non-finite numbers as `null`, and a
 * TypeError for cycles and BigInts.
 *
 * The digest is SHA-256 over fixed-size UTF-8 chunks (cut so a chunk never ends
 * inside a surrogate pair, which keeps the encoding lossless), then SHA-256 of
 * the chunk digests and the text's length. Equal texts give equal digests
 * whichever way they were produced, a whole string or pieces.
 */

const SLICE_MS = 4;
/** Containers this close to the root are always walked here. */
const WALK_DEPTH = 6;
/** Arrays and objects with more members are walked here at any depth. */
const WALK_MEMBERS = 64;
/** UTF-16 units per digest chunk. */
export const DIGEST_CHUNK = 1 << 22;
const EMIT_CHARS = 1 << 16;

export type Pause = () => Promise<void>;
const nextTask: Pause = () => new Promise<void>(resolve => setTimeout(resolve, 0));

type Frame = {
  holder: any;
  /** Object member names; null for an array. */
  keys: string[] | null;
  index: number;
  length: number;
  open: boolean;
  /** Whether an object member has been written (the next one needs a comma). */
  wrote: boolean;
  depth: number;
};

/** Array elements serialized by one native call when no element has a `toJSON` of its own. */
const BATCH = 256;
const hooked = (value: unknown) => value !== null && typeof value === 'object' && typeof (value as any).toJSON === 'function';

/** The value SerializeJSONProperty serializes for `holder[key]`, before its kind is decided. */
function prepare(holder: any, key: string): unknown {
  let value = holder[key];
  if (value !== null && (typeof value === 'object' || typeof value === 'bigint')) {
    const toJSON = (value as any).toJSON;
    if (typeof toJSON === 'function') value = toJSON.call(value, key);
  }
  if (value !== null && typeof value === 'object') {
    if (value instanceof Number) value = Number(value);
    else if (value instanceof String) value = String(value);
    else if (value instanceof Boolean) value = value.valueOf();
  }
  return value;
}

/** Emits `JSON.stringify(value)` in order; resolves once the last piece was emitted. */
export async function serializeInSlices(
  value: unknown,
  emit: (text: string) => void | Promise<void>,
  pause: Pause = nextTask,
): Promise<void> {
  const parts: string[] = [];
  let partChars = 0, deadline = performance.now() + SLICE_MS, steps = 0;
  const ancestors = new Set<object>();
  const stack: Frame[] = [];
  const write = async (text: string) => {
    parts.push(text); partChars += text.length;
    if (partChars >= EMIT_CHARS) {
      const joined = parts.join(''); parts.length = 0; partChars = 0;
      await emit(joined);
    }
  };
  /* One prepared value: its text, a frame to walk, or undefined (no JSON form). */
  const visit = (prepared: unknown, depth: number): string | Frame | undefined => {
    if (prepared === null) return 'null';
    switch (typeof prepared) {
      case 'string': return JSON.stringify(prepared);
      case 'number': return Number.isFinite(prepared) ? JSON.stringify(prepared) : 'null';
      case 'boolean': return prepared ? 'true' : 'false';
      case 'bigint': throw new TypeError('Do not know how to serialize a BigInt');
      case 'object': break;
      default: return undefined;
    }
    const object = prepared as any;
    const keys = Array.isArray(object) ? null : Object.keys(object);
    const length = keys ? keys.length : object.length;
    // A value whose toJSON already ran must not run another (a native
    // JSON.stringify of it would): such a value is walked here.
    if (!hooked(object) && depth >= WALK_DEPTH && length <= WALK_MEMBERS) return JSON.stringify(object);
    if (ancestors.has(object)) throw new TypeError('Converting circular structure to JSON');
    ancestors.add(object);
    return { holder: object, keys, index: 0, length, open: false, wrote: false, depth };
  };
  const enter = async (result: string | Frame) => {
    if (typeof result === 'string') await write(result);
    else stack.push(result);
  };

  const root = visit(prepare({ '': value }, ''), 0);
  if (root === undefined) throw new TypeError('The value has no JSON form');
  await enter(root);
  while (stack.length) {
    const frame = stack.at(-1)!;
    if (!frame.open) { await write(frame.keys ? '{' : '['); frame.open = true; }
    if (frame.index >= frame.length) {
      await write(frame.keys ? '}' : ']');
      ancestors.delete(frame.holder);
      stack.pop();
    } else if (frame.keys) {
      const key = frame.keys[frame.index++]!;
      const result = visit(prepare(frame.holder, key), frame.depth + 1);
      if (result !== undefined) {
        await write(`${frame.wrote ? ',' : ''}${JSON.stringify(key)}:`);
        frame.wrote = true;
        await enter(result);
      }
    } else {
      const index = frame.index;
      const batch = frame.depth >= WALK_DEPTH ? frame.holder.slice(index, Math.min(frame.length, index + BATCH)) : null;
      if (batch && !batch.some(hooked)) {
        // Elements without a toJSON of their own serialize the same way in a
        // native call over a slice (holes and undefined as null).
        await write(`${index > 0 ? ',' : ''}${JSON.stringify(batch).slice(1, -1)}`);
        frame.index += batch.length;
      } else {
        frame.index++;
        if (index > 0) await write(',');
        await enter(visit(prepare(frame.holder, String(index)), frame.depth + 1) ?? 'null');
      }
    }
    if (++steps % 16 === 0 && performance.now() >= deadline) {
      await pause();
      deadline = performance.now() + SLICE_MS;
    }
  }
  if (parts.length) await emit(parts.join(''));
}

const encoder = new TextEncoder();
const isHigh = (code: number) => code >= 0xd800 && code <= 0xdbff;

function hex(bytes: ArrayBuffer): string {
  return [...new Uint8Array(bytes)].map(byte => byte.toString(16).padStart(2, '0')).join('');
}

/** Incremental digest of JSON text pushed in pieces (see the module comment). */
export class TextDigest {
  private buffer = '';
  private length = 0;
  private readonly digests: Promise<ArrayBuffer>[] = [];

  async push(text: string): Promise<void> {
    this.length += text.length;
    this.buffer += text;
    while (this.buffer.length > DIGEST_CHUNK) {
      let end = DIGEST_CHUNK;
      if (isHigh(this.buffer.charCodeAt(end - 1))) end++;
      if (end > this.buffer.length) return;
      await this.chunk(this.buffer.slice(0, end));
      this.buffer = this.buffer.slice(end);
    }
  }

  async finish(): Promise<string> {
    if (this.buffer.length || !this.digests.length) await this.chunk(this.buffer);
    this.buffer = '';
    const parts = await Promise.all(this.digests);
    const joined = new Uint8Array(parts.length * 32 + 8);
    parts.forEach((part, index) => joined.set(new Uint8Array(part), index * 32));
    new DataView(joined.buffer).setFloat64(parts.length * 32, this.length);
    return hex(await crypto.subtle.digest('SHA-256', joined));
  }

  private async chunk(text: string): Promise<void> {
    const pending = crypto.subtle.digest('SHA-256', encoder.encode(text));
    this.digests.push(pending);
    // One chunk in flight at a time bounds the encoded copies held at once.
    await pending;
  }
}

/** The digest of a whole string, encoded a chunk per task. */
export async function digestText(text: string, pause: Pause = nextTask): Promise<string> {
  const digest = new TextDigest();
  for (let offset = 0; offset < text.length;) {
    let end = Math.min(text.length, offset + DIGEST_CHUNK);
    if (end < text.length && isHigh(text.charCodeAt(end - 1))) end++;
    await digest.push(text.slice(offset, end));
    offset = end;
    if (offset < text.length) await pause();
  }
  return digest.finish();
}

/** The digest of `JSON.stringify(value)`, computed in slices without building the string. */
export async function digestJSON(value: unknown, pause: Pause = nextTask): Promise<{ digest: string; length: number }> {
  const digest = new TextDigest();
  let length = 0;
  await serializeInSlices(value, text => { length += text.length; return digest.push(text); }, pause);
  return { digest: await digest.finish(), length };
}
