/** Serialize plain project data in bounded slices, allowing input and paint between slices. */
export async function stringifyAsync(value: unknown, yieldTask = () => new Promise<void>(resolve => setTimeout(resolve, 0))): Promise<string> {
  type Task = { value: any } | { text: string } | { leave: object } | { string: string; offset: number };
  const tasks: Task[] = [{ value }], ancestors = new Set<object>(), chunks: string[] = [], parts: string[] = [];
  let deadline = performance.now() + 4, steps = 0;
  while (tasks.length) {
    const task = tasks.pop()!;
    if ('string' in task) {
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
        ancestors.add(item);
        tasks.push({ leave: item });
        const array = Array.isArray(item);
        const keys = array ? null : Object.keys(item).filter(key => item[key] !== undefined && typeof item[key] !== 'function' && typeof item[key] !== 'symbol');
        chunks.push(array ? '[' : '{');
        tasks.push({ text: array ? ']' : '}' });
        const count = array ? item.length : keys!.length;
        for (let i = count - 1; i >= 0; i--) {
          if (i < count - 1) tasks.push({ text: ',' });
          const key = array ? i : keys![i]!;
          tasks.push({ value: item[key] });
          if (!array) tasks.push({ text: JSON.stringify(key) + ':' });
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
