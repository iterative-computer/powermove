import { yieldTask as yieldToBrowser } from './yield-task';
/** Serialize plain project data in bounded slices, allowing input and paint between slices. */
export async function stringifyAsync(value: unknown, yieldTask = yieldToBrowser): Promise<string> {
  type Task = { value: any } | { text: string } | { leave: object } | { string: string; offset: number } | { entries: any; keys: string[] | null; index: number; count: number };
  const tasks: Task[] = [{ value }], ancestors = new Set<object>(), chunks: string[] = [], parts: string[] = [];
  let deadline = performance.now() + 4, steps = 0;
  // Small keyframe/property leaves are cheap for the native serializer. Keep
  // strings and arrays bounded so this fast path cannot become one large task.
  const primitive = (item: any) => item === null || (typeof item !== 'object' && typeof item !== 'bigint'
    && (typeof item !== 'string' || item.length <= 1024));
  const smallArray = (item: any) => Array.isArray(item) && item.length <= 16 && !('toJSON' in item)
    && item.every(value => typeof value !== 'function' && primitive(value));
  while (tasks.length) {
    const task = tasks.pop()!;
    if ('entries' in task) {
      // One child at a time. Expanding a million-keyframe array into millions
      // of pending task objects before the next yield defeats the slice budget.
      if (task.index < task.count) {
        const index = task.index++;
        tasks.push(task);
        if (index) chunks.push(',');
        const key = task.keys ? task.keys[index]! : index;
        if (task.keys) chunks.push(JSON.stringify(key) + ':');
        tasks.push({ value: task.entries[key] });
      }
    } else if ('string' in task) {
      let end = Math.min(task.string.length, task.offset + 16_384);
      const high = task.string.charCodeAt(end - 1), low = task.string.charCodeAt(end);
      if (high >= 0xd800 && high <= 0xdbff && low >= 0xdc00 && low <= 0xdfff) end--;
      chunks.push(JSON.stringify(task.string.slice(task.offset, end)).slice(1, -1));
      if (end < task.string.length) tasks.push({ string: task.string, offset: end });
      else chunks.push('"');
    } else if ('text' in task) chunks.push(task.text);
    else if ('leave' in task) ancestors.delete(task.leave);
    else {
      const item = task.value;
      if (typeof item === 'string' && item.length > 16_384) { chunks.push('"'); tasks.push({ string: item, offset: 0 }); }
      else if (item === null || typeof item !== 'object') chunks.push(JSON.stringify(item) ?? 'null');
      else {
        if (ancestors.has(item)) throw new TypeError('Cannot save circular project data');
        const array = Array.isArray(item);
        const ownKeys = array ? null : Object.keys(item);
        const keys = ownKeys?.filter(key => item[key] !== undefined && typeof item[key] !== 'function' && typeof item[key] !== 'symbol');
        const prototype = array ? null : Object.getPrototypeOf(item);
        const native = array ? smallArray(item) : keys!.length <= 16
          && (prototype === Object.prototype || prototype === null) && !('toJSON' in item)
          // Native JSON can call a function value's toJSON before omitting it.
          // Project object fields have always omitted functions without hooks.
          && ownKeys!.every(key => typeof item[key] !== 'function')
          && keys!.every(key => primitive(item[key]) || smallArray(item[key]));
        if (native) chunks.push(JSON.stringify(item));
        else {
          ancestors.add(item);
          tasks.push({ leave: item });
          chunks.push(array ? '[' : '{');
          tasks.push({ text: array ? ']' : '}' });
          const count = array ? item.length : keys!.length;
          tasks.push({ entries: item, keys: keys || null, index: 0, count });
        }
      }
    }
    if (chunks.length >= 4096) { parts.push(chunks.join('')); chunks.length = 0; }
    if (++steps % 32 === 0 && performance.now() >= deadline) {
      await yieldTask();
      deadline = performance.now() + 4;
    }
  }
  parts.push(chunks.join(''));
  return parts.join('');
}
