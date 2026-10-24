import { compareJsonStrings, JsonBudget, jsonCloneCanonicalAsync, jsonOwnKeys, type JsonAsyncOptions } from '../../../shared/canonical-async';

const own = (value: object, key: string) => Object.prototype.hasOwnProperty.call(value, key);
type Frame = { before: any; after: any; array: boolean; index: number; count: number; keys?: Iterator<string>; beforeKeys?: Iterator<string> };

/** Compare already prepared JSON trees without serialization or recursion. */
async function plainEqual(before: any, after: any, budget: JsonBudget): Promise<boolean> {
  const pending: Frame[] = [];
  let b = before, a = after;
  for (;;) {
    const equalStrings = typeof a === 'string' && typeof b === 'string' ? (a.length <= 1024 && b.length <= 1024 ? a === b : await compareJsonStrings(a, b, budget) === 0) : undefined;
    if (equalStrings === false) return false;
    if (equalStrings !== true && b !== a) {
      if (a === null || b === null || typeof a !== 'object' || typeof b !== 'object') return false;
      const array = Array.isArray(a);
      if (Array.isArray(b) !== array || (array && b.length !== a.length)) return false;
      pending.push({ before: b, after: a, array, index: 0, count: 0, keys: array ? undefined : jsonOwnKeys(a), beforeKeys: array ? undefined : jsonOwnKeys(b) });
    }
    let found = false;
    while (pending.length) {
      if (budget.due()) await budget.yield();
      const frame = pending[pending.length - 1]!;
      if (frame.array) {
        if (frame.index < frame.after.length) {
          b = frame.before[frame.index]; a = frame.after[frame.index++]; found = true; break;
        }
      } else {
        const next = frame.keys!.next();
        if (!next.done) {
          if (!own(frame.before, next.value)) return false;
          frame.count++;
          b = frame.before[next.value]; a = frame.after[next.value]; found = true; break;
        }
        const previous = frame.beforeKeys!.next();
        if (!previous.done) { if (--frame.count < 0) return false; continue; }
        if (frame.count) return false;
      }
      pending.pop();
    }
    if (!found) return true;
  }
}

/**
 * Cooperative sameJson: `before` is the engine's plain JSON shadow, `after`
 * is live JavaScript. Strict aliases are accepted exactly as in sameJson;
 * hooks are applied once at their actual member key, including hooks that
 * return themselves or another hook-bearing object. Hook subtrees are fully
 * validated before comparing, so a circular hook result still throws even
 * when an earlier member differs. No whole-value stringify is used.
 *
 * This observes a traversal across tasks, not an atomic live-object read.
 * A caller must generation-fence announced edits and retry on change. No
 * asynchronous traversal can prove the absence of unannounced raw writes
 * to a member visited before its last await; callers must not treat this as
 * such a proof. Another asynchronous pass cannot close that race.
 */
export async function sameJsonAsync(before: any, after: any, options: JsonAsyncOptions = {}): Promise<boolean> {
  const budget = new JsonBudget(options.yieldTask);
  const frames: Frame[] = [];
  const ancestors = new Set<object>();
  let b = before, a = after, key = '', arraySlot = false;
  for (;;) {
    const equalStrings = typeof a === 'string' && typeof b === 'string' ? (a.length <= 1024 && b.length <= 1024 ? a === b : await compareJsonStrings(a, b, budget) === 0) : undefined;
    if (equalStrings === false) return false;
    if (equalStrings !== true && b !== a) {
      const hooked = a !== null && typeof a === 'object' && (typeof a.toJSON === 'function' || a instanceof Number || a instanceof String || a instanceof Boolean);
      if (hooked) {
        const clonedAfter = await jsonCloneCanonicalAsync({ [key]: a }, options);
        if (arraySlot && !own(clonedAfter, key)) { if (b !== null) return false; }
        else {
          const clonedBefore = await jsonCloneCanonicalAsync({ [key]: b }, options);
          if (!await plainEqual(clonedBefore, clonedAfter, budget)) return false;
        }
      } else {
        if (typeof a === 'number') { if (Number.isFinite(a) || b !== null) return false; }
        else if (a === null || typeof a !== 'object') return false;
        else {
          if (b === null || typeof b !== 'object') return false;
          const array = Array.isArray(a);
          if (array !== Array.isArray(b) || (array && a.length !== b.length)) return false;
          if (ancestors.has(a)) throw new TypeError('Converting circular structure to JSON');
          ancestors.add(a);
          frames.push({ before: b, after: a, array, index: 0, count: 0, keys: array ? undefined : jsonOwnKeys(a), beforeKeys: array ? undefined : jsonOwnKeys(b) });
        }
      }
    }
    let found = false;
    while (frames.length) {
      if (budget.due()) await budget.yield();
      const frame = frames[frames.length - 1]!;
      if (frame.array) {
        if (frame.index < frame.after.length) {
          key = String(frame.index++);
          b = frame.before[key]; a = frame.after[key]; arraySlot = true;
          if (a === undefined || typeof a === 'function' || typeof a === 'symbol') a = null;
          found = true; break;
        }
      } else {
        const next = frame.keys!.next();
        if (!next.done) {
          key = next.value;
          const live = frame.after[key];
          if (live === undefined || typeof live === 'function' || typeof live === 'symbol') continue;
          // An absent hook result is an omitted member, including a member
          // not present in the shadow. Clone its wrapper once to preserve
          // both omission and hook/cycle validation semantics.
          if (typeof live !== 'string' && own(frame.before, key) && frame.before[key] === live) { frame.count++; continue; }
          if (live !== null && typeof live === 'object' && typeof live.toJSON === 'function') {
            const cloned = await jsonCloneCanonicalAsync({ [key]: live }, options);
            if (!own(cloned, key)) continue;
            if (!own(frame.before, key)) return false;
            const previous = await jsonCloneCanonicalAsync({ [key]: frame.before[key] }, options);
            if (!await plainEqual(previous, cloned, budget)) return false;
            frame.count++;
            continue;
          }
          if (!own(frame.before, key)) return false;
          frame.count++;
          b = frame.before[key]; a = live; arraySlot = false;
          found = true; break;
        }
        const previous = frame.beforeKeys!.next();
        if (!previous.done) { if (--frame.count < 0) return false; continue; }
        if (frame.count) return false;
      }
      ancestors.delete(frame.after);
      frames.pop();
    }
    if (!found) return true;
  }
}
