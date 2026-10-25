/* Local incremental integrity digest, byte-for-byte with pm-model/consistency.rs.
 * The canonical wire SHA-256 is unchanged. Only JSON shadows enter this cache;
 * live mutable aliases are compared against the shadow before using it. */
import { compareJsonStrings, JsonBudget } from '../../../shared/canonical-async';
import type { DigestInstruction } from '../../../shared/document-digest';
import { canonicalHash } from '../../../shared/canonical-json';
import type { PatchOp } from '../../../shared/patch-ids';

export class ConsistencyHash {
  private cache = new WeakMap<object, Map<number, string>>();
  private epoch = 0;
  hashed = 0;
  reused = 0;

  /** applyOps mutates containers in place. Invalidate all existing ancestors
   * before any operation in a batch (including order, insert and delete). */
  invalidate(root: any, ops: readonly PatchOp[]): void {
    this.epoch++;
    for (const op of ops) {
      let node = root;
      if (node && typeof node === 'object') this.cache.delete(node);
      for (const part of op.path) {
        node = node?.[part];
        if (!node || typeof node !== 'object') break;
        this.cache.delete(node);
      }
    }
  }

  digest(root: any): string {
    this.hashed = this.reused = 0;
    return root === undefined ? '' : this.visit(root, 0);
  }
  /** Batch independent small hashes in postorder. The async cache is guarded
   * by invalidate's epoch: a hash started before an edit cannot repopulate
   * caches that the edit already invalidated. Callers still revision-fence
   * the returned digest because the shadow may change across awaits. */
  async digestAsync(
    root: any,
    hash: (value: any, prefix?: string) => Promise<string>,
    batch?: (instructions: DigestInstruction[]) => Promise<string[]>,
  ): Promise<string> {
    this.hashed = this.reused = 0;
    if (root === undefined) return '';
    const epoch = this.epoch;
    const budget = new JsonBudget();
    type Ref = string | number;
    type Frame = { node: any; depth: number; array: boolean; index: number; length: number; keys?: string[]; children: Ref[]; members: Array<[string, Ref]>; key: string };
    type CacheWrite = { node: object; depth: number; index: number };
    let instructions: DigestInstruction[] = [], bytes = 0;
    let fixups: Array<{ owner: any[]; index: number }> = [];
    let cacheWrites: CacheWrite[] = [];
    const cache = (node: any, depth: number, digest: string) => {
      if (this.epoch !== epoch || node === null || typeof node !== 'object') return;
      let depths = this.cache.get(node);
      if (!depths) this.cache.set(node, depths = new Map());
      depths.set(depth, digest);
    };
    const flush = async (): Promise<string[]> => {
      if (!instructions.length) return [];
      let hashes: string[];
      if (batch) hashes = await batch(instructions);
      else {
        hashes = [];
        const resolve = (ref: Ref) => typeof ref === 'number' ? hashes[ref]! : ref;
        for (const instruction of instructions) {
          if ('value' in instruction) hashes.push(await hash(instruction.value, instruction.prefix));
          else if ('array' in instruction) hashes.push(await hash(['a', instruction.array.map(resolve)]));
          else hashes.push(await hash(['o', instruction.object.map(([key, ref]) => [key, resolve(ref)])]));
        }
      }
      if (hashes.length !== instructions.length || hashes.some(value => typeof value !== 'string' || !/^[0-9a-f]{64}$/.test(value))) throw new TypeError('Invalid digest batch response');
      for (const { owner, index } of fixups) owner[index] = hashes[owner[index] as number]!;
      for (const write of cacheWrites) cache(write.node, write.depth, hashes[write.index]!);
      instructions = []; bytes = 0; fixups = []; cacheWrites = [];
      return hashes;
    };
    const work = function* (self: ConsistencyHash): Generator<Promise<any>, string, any> {
      const frames: Frame[] = [];
      let node = root, depth = 0;
      for (;;) {
        if (budget.due()) yield budget.yield();
        const container = node !== null && typeof node === 'object';
        const cached = container ? self.cache.get(node)?.get(depth) : undefined;
        let result: Ref | undefined = cached;
        if (cached !== undefined) self.reused++;
        else {
          self.hashed++;
          const array = Array.isArray(node);
          let bulk = false;
          if (array && node.length > 64) {
            for (let i = 0; i < node.length; i++) {
              const item = node[i];
              if (!item || typeof item !== 'object' || typeof item.id !== 'string') { bulk = true; break; }
              if (budget.due()) yield budget.yield();
            }
          }
          if (depth >= 6 || !container || bulk) {
            const estimate = yield* smallJsonBudget(node, 32 * 1024, budget);
            if (estimate >= 32 * 1024) {
              yield flush();
              result = yield hash(node, 'v');
              cache(node, depth, result as string);
            } else {
              if (instructions.length >= 128 || bytes + estimate + 64 > 256 * 1024) yield flush();
              result = instructions.length;
              instructions.push({ value: node, prefix: 'v' }); bytes += estimate + 64;
              if (container) cacheWrites.push({ node, depth, index: result });
            }
          } else {
            const keys = array ? undefined : yield* digestKeys(node, budget);
            frames.push({ node, depth, array, index: 0, length: array ? node.length : keys!.length, keys, children: [], members: [], key: '' });
          }
        }
        let complete = false;
        while (frames.length) {
          if (budget.due()) yield budget.yield();
          const frame = frames[frames.length - 1]!;
          if (result !== undefined) {
            const owner = frame.array ? frame.children : [frame.key, result];
            const index = frame.array ? frame.children.length : 1;
            if (frame.array) frame.children.push(result);
            else frame.members.push(owner as [string, Ref]);
            if (typeof result === 'number') fixups.push({ owner, index });
            result = undefined;
          }
          if (frame.index < frame.length) {
            frame.key = frame.array ? String(frame.index++) : frame.keys![frame.index++]!;
            node = frame.node[frame.key]; depth = frame.depth + 1;
            break;
          }
          let estimate = frame.array ? 32 + frame.children.length * 67 : 32;
          if (!frame.array) {
            for (const [key] of frame.members) {
              estimate += key.length * 6 + 76;
              if (estimate > 256 * 1024) break;
              if (budget.due()) yield budget.yield();
            }
          }
          if (estimate > 256 * 1024) {
            yield flush();
            result = yield hash(frame.array ? ['a', frame.children] : ['o', frame.members]);
            cache(frame.node, frame.depth, result as string);
          } else {
            if (instructions.length >= 128 || bytes + estimate > 256 * 1024) yield flush();
            result = instructions.length;
            instructions.push(frame.array ? { array: frame.children } : { object: frame.members }); bytes += estimate;
            cacheWrites.push({ node: frame.node, depth: frame.depth, index: result });
          }
          frames.pop();
          if (!frames.length) { complete = true; break; }
        }
        if (result !== undefined && (!frames.length || complete)) {
          if (typeof result === 'number') { const index = result; result = (yield flush())[index]; }
          return result as string;
        }
      }
    }(this);
    let step = work.next();
    while (!step.done) step = work.next(await step.value);
    return step.value;
  }

  private visit(value: any, depth: number): string {
    const container = value !== null && typeof value === 'object';
    const cached = container ? this.cache.get(value)?.get(depth) : undefined;
    if (cached !== undefined) { this.reused++; return cached; }
    this.hashed++;
    let digest: string;
    const bulk = Array.isArray(value) && value.length > 64 && !value.every(item => item && typeof item === 'object' && typeof item.id === 'string');
    if (depth >= 6 || !container || bulk) digest = canonicalHash(value, 'v');
    else if (Array.isArray(value)) digest = canonicalHash(['a', value.map(child => this.visit(child, depth + 1))]);
    else digest = canonicalHash(['o', Object.keys(value).sort().map(key => [key, this.visit(value[key], depth + 1)])]);
    if (container) {
      let depths = this.cache.get(value);
      if (!depths) this.cache.set(value, depths = new Map());
      depths.set(depth, digest);
    }
    return digest;
  }
}

/** Conservative JSON byte census. It stops at the leaf budget, so a huge
 * scalar or bulk array never becomes an unbounded stringify/size pass. */
function* smallJsonBudget(root: any, limit: number, budget: JsonBudget): Generator<Promise<any>, number, any> {
  type Cursor = { value: any; index: number; array: boolean; keys?: Iterator<string> };
  const stack: Cursor[] = [];
  let value = root, bytes = 0;
  for (;;) {
    if (budget.due()) yield budget.yield();
    if (typeof value === 'string') bytes += value.length * 6 + 2;
    else if (value === null) bytes += 4;
    else if (typeof value === 'number') bytes += 32;
    else if (typeof value === 'boolean') bytes += 5;
    else if (value && typeof value === 'object') {
      bytes += 2;
      const array = Array.isArray(value);
      stack.push({ value, index: 0, array, keys: array ? undefined : ownKeys(value) });
    } else bytes += 32;
    if (bytes >= limit) return limit;
    let found = false;
    while (stack.length) {
      const frame = stack[stack.length - 1]!;
      if (frame.array && frame.index < frame.value.length) {
        bytes++; value = frame.value[frame.index++]; found = true; break;
      }
      if (!frame.array) {
        const key = frame.keys!.next();
        if (!key.done) {
          bytes += key.value.length * 6 + 4;
          if (bytes >= limit) return limit;
          value = frame.value[key.value]; found = true; break;
        }
      }
      stack.pop();
    }
    if (!found) return bytes;
  }
}

function* ownKeys(value: object): Generator<string> {
  for (const key in value) if (Object.hasOwn(value, key)) yield key;
}

function* digestKeys(value: object, budget: JsonBudget): Generator<Promise<any>, string[], any> {
  let keys: string[] = [];
  let short = true;
  for (const key of ownKeys(value)) {
    keys.push(key);
    if (key.length > 128 || keys.length > 64) short = false;
    if (budget.due()) yield budget.yield();
  }
  if (short) return keys.sort();
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
