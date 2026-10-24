import { yieldTask as yieldToBrowser } from './yield-task';
/** Parse saved JSON in small slices so large projects do not monopolize input. */
export async function parseAsync(source: string, yieldTask = yieldToBrowser): Promise<any> {
  // Native JSON parsing is much cheaper for individual layers/keyframes. Bound
  // each native task so a large document still gives input and paint a turn.
  const nativeLimit = 16_384;
  if (source.length <= nativeLimit) return JSON.parse(source);
  let offset = 0, checkpoint = 0, deadline = performance.now() + 4;
  const fail = (): never => { throw new SyntaxError(`Invalid JSON at position ${offset}`); };
  const whitespace = (code: number) => code === 32 || code === 9 || code === 10 || code === 13;
  function* step(): Generator<void> {
    if (offset - checkpoint >= 2048) { checkpoint = offset; yield; }
  }
  function* space(): Generator<void> {
    while (offset < source.length && whitespace(source.charCodeAt(offset))) {
      offset++;
      if (offset - checkpoint >= 2048) yield* step();
    }
  }
  function* string(): Generator<void, string> {
    const start = offset++;
    while (offset < source.length) {
      // Media payloads are usually long, unescaped strings. Scan bounded
      // blocks in native code instead of creating a generator per character.
      const end = Math.min(source.length, offset + nativeLimit);
      const special = source.slice(offset, end).search(/["\\\u0000-\u001f]/);
      if (special < 0) offset = end;
      else {
        offset += special;
        const code = source.charCodeAt(offset++);
        // Decode through JSON.parse even without escapes: returning a slice
        // can keep the entire document alive through one retained string.
        if (code === 34) return JSON.parse(source.slice(start, offset));
        if (code !== 92) return fail();
        offset++; // The escaped character cannot close the string.
      }
      if (offset - checkpoint >= 2048) yield* step();
    }
    return fail();
  }
  // Find a complete small subtree without allocating parser tasks for each
  // property. JSON.parse validates its syntax, escapes, duplicate keys and
  // special own-property names. Large containers use the cooperative path.
  function smallContainerEnd(): number {
    const limit = Math.min(source.length, offset + nativeLimit);
    let depth = 0, quoted = false, escaped = false;
    for (let index = offset; index < limit; index++) {
      const code = source.charCodeAt(index);
      if (quoted) {
        if (escaped) escaped = false;
        else if (code === 92) escaped = true;
        else if (code === 34) quoted = false;
      } else if (code === 34) quoted = true;
      else if (code === 123 || code === 91) depth++;
      else if ((code === 125 || code === 93) && --depth === 0) return index + 1;
    }
    return -1;
  }
  function* value(): Generator<void, any> {
    yield* space();
    const char = source[offset];
    if (char === '"') return yield* string();
    if (char === '{' || char === '[') {
      // Lookahead scans without advancing offset. Account for that work even
      // when large nested containers cannot take the native parsing path.
      const nativeEnd = smallContainerEnd();
      yield;
      if (nativeEnd >= 0) {
        const result = JSON.parse(source.slice(offset, nativeEnd));
        offset = nativeEnd;
        yield* step();
        return result;
      }
      offset++;
      const array = char === '[', result: any = array ? [] : {}, end = array ? ']' : '}';
      yield* space();
      if (source[offset] === end) { offset++; return result; }
      while (offset < source.length) {
        if (offset - checkpoint >= 2048) yield* step();
        let key: string | undefined;
        if (!array) {
          if (source[offset] !== '"') return fail();
          key = yield* string();
          yield* space();
          if (source[offset++] !== ':') return fail();
        }
        const item = yield* value();
        if (array) result.push(item);
        // JSON.parse treats __proto__ as an ordinary own property.
        else Object.defineProperty(result, key!, { value: item, enumerable: true, configurable: true, writable: true });
        yield* space();
        const separator = source[offset++];
        if (separator === end) return result;
        if (separator !== ',') return fail();
        yield* space();
      }
      return fail();
    }
    const start = offset;
    while (offset < source.length) {
      const code = source.charCodeAt(offset);
      if (whitespace(code) || code === 44 || code === 93 || code === 125) break;
      offset++;
      if (offset - checkpoint >= 2048) yield* step();
    }
    if (start === offset) return fail();
    return JSON.parse(source.slice(start, offset));
  }
  const parser = value();
  let next = parser.next();
  while (!next.done) {
    if (performance.now() >= deadline) { await yieldTask(); deadline = performance.now() + 4; }
    next = parser.next();
  }
  // Consume trailing whitespace through the same bounded scanner.
  for (const _ of space()) {
    if (performance.now() >= deadline) { await yieldTask(); deadline = performance.now() + 4; }
  }
  if (offset !== source.length) return fail();
  return next.value;
}
