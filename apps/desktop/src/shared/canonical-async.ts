/** Cooperative JSON preparation. Callers must fence mutations across awaits.
 * Getters, hooks, proxy traps and the VM's own-key enumeration are indivisible;
 * cooperative scheduling cannot put a time limit on user supplied code.
 */
export interface JsonAsyncOptions { yieldTask?: () => Promise<void> }
export interface CanonicalChunksOptions extends JsonAsyncOptions { chunkBytes?: number }

type TaskPort = MessagePort & { ref?(): void; unref?(): void };
let tasks: { channel: MessageChannel; queue: Array<(() => void) | undefined>; head: number } | undefined;

function yieldJsonTask(): Promise<void> {
  if (typeof MessageChannel !== 'function') return new Promise(resolve => setTimeout(resolve, 0));
  if (!tasks) {
    const channel = new MessageChannel();
    const state = { channel, queue: [] as Array<(() => void) | undefined>, head: 0 };
    channel.port1.onmessage = () => {
      const resolve = state.queue[state.head];
      state.queue[state.head++] = undefined; // Never retain a completed task's closures.
      if (state.head === state.queue.length) {
        state.queue = []; state.head = 0;
        (channel.port1 as TaskPort).unref?.();
        (channel.port2 as TaskPort).unref?.();
      } else if (state.head >= 1024 && state.head * 2 >= state.queue.length) {
        state.queue = state.queue.slice(state.head); state.head = 0;
      }
      resolve?.();
    };
    // Browsers keep the shared task source. Node must stay alive while work
    // is pending, and may exit normally once the FIFO has drained.
    (channel.port1 as TaskPort).unref?.();
    (channel.port2 as TaskPort).unref?.();
    tasks = state;
  }
  const state = tasks;
  return new Promise(resolve => {
    if (state.head === state.queue.length) {
      (state.channel.port1 as TaskPort).ref?.();
      (state.channel.port2 as TaskPort).ref?.();
    }
    state.queue.push(resolve);
    // One callback per posted message: concurrent traversals take turns at
    // real task boundaries without the browser's nested timer clamp.
    state.channel.port2.postMessage(null);
  });
}

export class JsonBudget {
  private deadline = performance.now() + 2;
  private operations = 0;
  constructor(private readonly task: () => Promise<void> = yieldJsonTask) {}
  due(): boolean { return ++this.operations >= 256 || performance.now() >= this.deadline; }
  async yield(): Promise<void> {
    await this.task();
    this.operations = 0;
    this.deadline = performance.now() + 2;
  }
}

export const JSON_ABSENT = Symbol('JSON absent');
/** Apply a member's hook once; never reapply a hook on its returned object. */
export function prepareJson(value: any, key: string): any {
  if (value && typeof value === 'object' && typeof value.toJSON === 'function') value = value.toJSON(key);
  if (value instanceof Number || value instanceof String || value instanceof Boolean) value = value.valueOf();
  switch (typeof value) {
    case 'undefined': case 'function': case 'symbol': return JSON_ABSENT;
    case 'number': return Number.isFinite(value) ? (value === 0 ? 0 : value) : null;
    case 'bigint': throw new TypeError('Do not know how to serialize a BigInt');
    default: return value;
  }
}

/** Does not materialize an array of every member of a wide object. */
export function* jsonOwnKeys(value: object): Generator<string> {
  for (const key in value) if (Object.prototype.hasOwnProperty.call(value, key)) yield key;
}

/** UTF-16 comparison in bounded pieces, including unusually large keys. */
export async function compareJsonStrings(a: string, b: string, budget: JsonBudget): Promise<number> {
  const length = Math.min(a.length, b.length);
  for (let offset = 0; offset < length; offset += 1024) {
    const aa = a.slice(offset, offset + Math.min(1024, length - offset));
    const bb = b.slice(offset, offset + Math.min(1024, length - offset));
    if (aa !== bb) return aa < bb ? -1 : 1;
    if (budget.due()) await budget.yield();
  }
  return a.length < b.length ? -1 : a.length > b.length ? 1 : 0;
}

// Cache only short, small shapes. Length-prefixed keys avoid delimiter
// collisions; a fixed entry limit bounds retention across unrelated documents.
const keyShapes = new Map<string, string[]>();
type Work = Promise<any>;

function* objectKeys(value: object, sorted: boolean, budget: JsonBudget): Generator<Work, string[], any> {
  let keys: string[] = [];
  let cacheable = true;
  for (const key in value) {
    if (Object.prototype.hasOwnProperty.call(value, key)) {
      keys.push(key);
      if (key.length > 128 || keys.length > 64) cacheable = false;
    }
    if (budget.due()) yield budget.yield();
  }
  if (!sorted || keys.length < 2) return keys;
  if (cacheable) {
    let shape = '';
    for (const key of keys) shape += key.length + ':' + key;
    const cached = keyShapes.get(shape);
    if (cached) return cached;
    keys.sort(); // At most 64 short keys, never a wide-object sort.
    if (keyShapes.size >= 1024) keyShapes.delete(keyShapes.keys().next().value!);
    keyShapes.set(shape, keys);
    return keys;
  }
  let other: string[] = [];
  for (let width = 1; width < keys.length; width *= 2) {
    for (let start = 0; start < keys.length; start += width * 2) {
      let a = start, b = Math.min(start + width, keys.length);
      const aEnd = b, bEnd = Math.min(b + width, keys.length);
      for (let out = start; out < bEnd; out++) {
        const left = keys[a], right = keys[b];
        const takeLeft = a < aEnd && (b >= bEnd || (left!.length <= 1024 && right!.length <= 1024 ? left! <= right! : (yield compareJsonStrings(left!, right!, budget)) <= 0));
        other[out] = takeLeft ? keys[a++]! : keys[b++]!;
        if (budget.due()) yield budget.yield();
      }
    }
    [keys, other] = [other, keys];
  }
  return keys;
}

type Frame = { source: any; target?: any; array: boolean; index: number; length: number; keys?: string[]; count: number };

/** jsonClone / a JSON round trip, without a whole-document string. Strings
 * are immutable and are shared. The result is a sampled traversal, not an
 * atomic snapshot of an object that is being mutated between tasks. */
export async function jsonCloneAsync(value: unknown, options: JsonAsyncOptions = {}): Promise<any> {
  return runClone(value, options, false);
}

/** @internal Canonical hook visitation order for sameJson's serialization fallback. */
export async function jsonCloneCanonicalAsync(value: unknown, options: JsonAsyncOptions = {}): Promise<any> {
  return runClone(value, options, true);
}

async function runClone(value: unknown, options: JsonAsyncOptions, sorted: boolean): Promise<any> {
  const work = cloneWork(value, sorted, new JsonBudget(options.yieldTask));
  let step = work.next();
  while (!step.done) step = work.next(await step.value);
  return step.value;
}

/** The synchronous cursor does all ordinary members in one quantum. Only
 * actual scheduler boundaries allocate promises; no await per primitive. */
function* cloneWork(value: unknown, sorted: boolean, budget: JsonBudget): Generator<Work, any, any> {
  const frames: Frame[] = [];
  const ancestors = new Set<object>();
  let next = prepareJson(value, '');
  if (next === JSON_ABSENT) return undefined;
  let root: any, destination: any, key = '';
  for (;;) {
    if (budget.due()) yield budget.yield();
    const object = next !== null && typeof next === 'object';
    const array = object && Array.isArray(next);
    const length = array ? next.length : 0;
    const copy = object ? array ? new Array(length) : {} : next;
    if (!destination) root = copy;
    else if (key === '__proto__') Object.defineProperty(destination, key, { value: copy, enumerable: true, configurable: true, writable: true });
    else destination[key] = copy;
    if (object) {
      if (ancestors.has(next)) throw new TypeError('Converting circular structure to JSON');
      ancestors.add(next);
      const keys = array ? undefined : yield* objectKeys(next, sorted, budget);
      frames.push({ source: next, target: copy, array, index: 0, length: array ? length : keys!.length, keys, count: 0 });
    }
    let found = false;
    while (frames.length) {
      if (budget.due()) yield budget.yield();
      const frame = frames[frames.length - 1]!;
      if (frame.index < frame.length) {
        key = frame.array ? String(frame.index++) : frame.keys![frame.index++]!;
        next = prepareJson(frame.source[key], key);
        if (next === JSON_ABSENT) { if (!frame.array) continue; next = null; }
        destination = frame.target;
        found = true;
        break;
      }
      ancestors.delete(frame.source);
      frames.pop();
    }
    if (!found) return root;
  }
}

/** Quote bounded pieces; a valid surrogate pair never crosses an encoding
 * boundary, while JSON.stringify escapes lone surrogates exactly as before. */
function* quoted(value: string): Generator<string> {
  if (value.length <= 1024) { yield JSON.stringify(value); return; }
  yield '"';
  for (let start = 0; start < value.length;) {
    let end = Math.min(start + 1024, value.length);
    const last = value.charCodeAt(end - 1);
    if (end < value.length && last >= 0xd800 && last <= 0xdbff && value.charCodeAt(end) >= 0xdc00 && value.charCodeAt(end) <= 0xdfff) end--;
    yield JSON.stringify(value.slice(start, end)).slice(1, -1);
    start = end;
  }
  yield '"';
}

/** Bounded synchronous canonical token cursor; its stack contains ancestors,
 * never every element of a large array. Promise tokens are actual task breaks. */
function* canonicalTokens(value: unknown, budget: JsonBudget): Generator<string | Work, void, any> {
  const frames: Frame[] = [];
  const ancestors = new Set<object>();
  let next = prepareJson(value, '');
  if (next === JSON_ABSENT) throw new TypeError('The value has no JSON representation');
  let prefix = '';
  for (;;) {
    if (budget.due()) yield budget.yield();
    if (next !== null && typeof next === 'object') {
      if (ancestors.has(next)) throw new TypeError('Converting circular structure to JSON');
      ancestors.add(next);
      const array = Array.isArray(next);
      const keys = array ? undefined : yield* objectKeys(next, true, budget);
      frames.push({ source: next, array, index: 0, length: array ? next.length : keys!.length, keys, count: 0 });
      yield prefix + (array ? '[' : '{');
    } else if (typeof next === 'string' && next.length > 1024) {
      if (prefix) yield prefix;
      for (const piece of quoted(next)) {
        yield piece;
        if (budget.due()) yield budget.yield();
      }
    } else yield prefix + JSON.stringify(next);
    let found = false;
    while (frames.length) {
      if (budget.due()) yield budget.yield();
      const frame = frames[frames.length - 1]!;
      if (frame.index < frame.length) {
        const key = frame.array ? String(frame.index++) : frame.keys![frame.index++]!;
        next = prepareJson(frame.source[key], key);
        if (next === JSON_ABSENT) { if (!frame.array) continue; next = null; }
        prefix = frame.count++ ? ',' : '';
        if (!frame.array) {
          if (key.length <= 1024) prefix += JSON.stringify(key) + ':';
          else {
            if (prefix) yield prefix;
            for (const piece of quoted(key)) {
              yield piece;
              if (budget.due()) yield budget.yield();
            }
            prefix = ':';
          }
        }
        found = true;
        break;
      }
      ancestors.delete(frame.source);
      frames.pop();
      yield frame.array ? ']' : '}';
    }
    if (!found) return;
  }
}

/** Canonical UTF-8 bytes, with no renderer SHA or whole-document serialization.
 * An absent root has no JSON representation and throws, as canonicalHash does.
 * Output chunks are at most chunkBytes (capped at 256 KiB); consumers must
 * decode as a stream. Text aggregation is bounded to approximately 16 KiB. */
export async function* canonicalChunks(value: unknown, options: CanonicalChunksOptions = {}): AsyncGenerator<Uint8Array> {
  const requested = options.chunkBytes ?? 64 * 1024;
  if (!Number.isFinite(requested) || requested < 1) throw new RangeError('chunkBytes must be positive');
  const size = Math.min(256 * 1024, Math.max(1, Math.floor(requested)));
  const budget = new JsonBudget(options.yieldTask);
  const encoder = new TextEncoder();
  const tokens = canonicalTokens(value, budget);
  let buffer = new Uint8Array(size), used = 0, text = '', result: any;
  let step = tokens.next();
  for (;;) {
    if (!step.done && typeof step.value !== 'string') {
      result = await step.value;
      step = tokens.next(result);
      continue;
    }
    if (!step.done) text += step.value;
    if (text.length >= 16 * 1024 || step.done) {
      const bytes = encoder.encode(text);
      text = '';
      for (let offset = 0; offset < bytes.length;) {
        const take = Math.min(size - used, bytes.length - offset);
        buffer.set(bytes.subarray(offset, offset + take), used);
        used += take;
        offset += take;
        if (used === size) { yield buffer; buffer = new Uint8Array(size); used = 0; }
        if (budget.due()) await budget.yield();
      }
    }
    if (step.done) break;
    step = tokens.next();
  }
  if (used) yield buffer.subarray(0, used);
}
