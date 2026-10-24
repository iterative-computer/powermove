import { compareJsonStrings, JsonBudget } from '../../../shared/canonical-async';

/** Bounded mismatch diagnostics. Equality and both digests check the entire
 * document; at most 256 paths are retained for a report. */
export async function changedPathsAsync(before: any, after: any): Promise<string[]> {
  const paths: string[] = [], path: Array<string | number> = [], budget = new JsonBudget();
  type Frame = { before: any; after: any; array: boolean; index: number; keys?: Generator<string>; pathLength: number };
  function* keys(a: any, b: any): Generator<string> {
    for (const key in a) if (Object.hasOwn(a, key)) yield key;
    for (const key in b) if (Object.hasOwn(b, key) && !Object.hasOwn(a, key)) yield key;
  }
  const frames: Frame[] = [];
  let a = before, b = after;
  for (;;) {
    const equal = typeof a === 'string' && typeof b === 'string' && (a.length > 1024 || b.length > 1024)
      ? await compareJsonStrings(a, b, budget) === 0 : a === b;
    if (!equal) {
      if (a && b && typeof a === 'object' && typeof b === 'object' && Array.isArray(a) === Array.isArray(b)) {
        const array = Array.isArray(a);
        frames.push({ before: a, after: b, array, index: 0, keys: array ? undefined : keys(a, b), pathLength: path.length });
      } else paths.push(path.map(String).join('.'));
    }
    if (paths.length >= 256) return paths;
    let found = false;
    while (frames.length) {
      if (budget.due()) await budget.yield();
      const frame = frames[frames.length - 1]!;
      path.length = frame.pathLength;
      const next = frame.array ? { done: frame.index >= Math.max(frame.before.length, frame.after.length), value: frame.index++ } : frame.keys!.next();
      if (!next.done) {
        const key = next.value!;
        path.push(key);
        a = frame.before[key]; b = frame.after[key];
        if (!Object.hasOwn(frame.before, key) || !Object.hasOwn(frame.after, key)) {
          paths.push(path.map(String).join('.'));
          if (paths.length >= 256) return paths;
          continue;
        }
        found = true; break;
      }
      frames.pop();
    }
    if (!found) return paths;
  }
}
