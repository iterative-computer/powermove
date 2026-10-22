import { describe, expect, it, vi } from 'vitest';
import { parseAsync } from './parse-async';

describe('cooperative JSON parsing', () => {
  it('matches native JSON including escaped strings and special property names', async () => {
    for (const json of [
      '{"__proto__":{"safe":true},"constructor":1,"duplicate":0,"duplicate":2}',
      JSON.stringify({ text: 'Quotes " and \\ and \n and 😀', values: [null, true, false, -3.2e10, {}, []] }),
      ' \n\t[1, 2, "\\u1234"] \r',
    ]) expect(await parseAsync(json)).toEqual(JSON.parse(json));
    expect(Object.getPrototypeOf(await parseAsync('{"__proto__":1}'))).toBe(Object.prototype);
  });
  it.each(['', '[1,]', '{"x":}', '{"x":1,}', '[1 2]', '{x:1}', '01', 'true false', '"unterminated', '"\\x"'])('rejects invalid JSON: %s', async json => {
    await expect(parseAsync(json)).rejects.toThrow(SyntaxError);
  });
  it('preserves strings, nesting and own properties across native parsing boundaries', async () => {
    for (const length of [16_380, 16_383, 16_384, 16_385, 32_768]) {
      const value = { padding: 'x'.repeat(length), layers: [
        { text: 'End quote " and escaped slash \\ and unicode 😀', nested: [[{ enabled: true }]] },
        JSON.parse('{"__proto__":{"safe":true},"constructor":1,"duplicate":0,"duplicate":2}'),
      ] };
      const source = JSON.stringify(value);
      const parsed = await parseAsync(source);
      expect(parsed).toEqual(JSON.parse(source));
      expect(Object.getPrototypeOf(parsed.layers[1])).toBe(Object.prototype);
      expect(Object.hasOwn(parsed.layers[1], '__proto__')).toBe(true);
    }
    const escaped = JSON.stringify('x'.repeat(16_382) + '\\"\n😀' + 'y'.repeat(16_384));
    expect(await parseAsync(escaped)).toBe(JSON.parse(escaped));
  });
  it('rejects invalid syntax after large strings and inside native subtrees', async () => {
    const padding = JSON.stringify('x'.repeat(20_000));
    for (const suffix of [', [1,]]', ', {"x":}]', ', {"x":1,}]', ', [1 2]]', ', 01]', ', true false]', '] trailing']) {
      const source = `[${padding}${suffix}`;
      expect(() => JSON.parse(source)).toThrow(SyntaxError);
      await expect(parseAsync(source)).rejects.toThrow(SyntaxError);
    }
    for (const invalid of ['\\x', '\\u12xz', '\n', '\\']) {
      await expect(parseAsync(`"${'x'.repeat(20_000)}${invalid}"`)).rejects.toThrow(SyntaxError);
    }
  });
  it('allows input callbacks to run before a large document finishes parsing', async () => {
    const source = JSON.stringify(Array.from({ length: 30_000 }, (_, index) => ({ index, name: `Layer ${index}`, values: [1, 2, 3] })));
    let yields = 0;
    const result = await parseAsync(source, async () => { yields++; });
    expect(yields).toBeGreaterThan(1);
    expect(result).toEqual(JSON.parse(source));
  });
  it('accounts for lookahead work when yielding through deeply nested containers', async () => {
    const source = '['.repeat(200) + JSON.stringify('x'.repeat(20_000)) + ']'.repeat(200);
    const charCodeAt = String.prototype.charCodeAt;
    let scanned = 0, firstYield = 0;
    const scan = vi.spyOn(String.prototype, 'charCodeAt').mockImplementation(function (this: string, index: number) {
      scanned++;
      return charCodeAt.call(this, index);
    });
    // Deterministic cost model: one bounded lookahead consumes a time slice.
    const now = vi.spyOn(performance, 'now').mockImplementation(() => scanned / 2048);
    try {
      const parsed = await parseAsync(source, async () => { firstYield ||= scanned; });
      expect(parsed).toEqual(JSON.parse(source));
      expect(Object.hasOwn(parsed, '0')).toBe(true);
      expect(firstYield).toBeGreaterThan(0);
      expect(firstYield).toBeLessThan(32_768);
    } finally { now.mockRestore(); scan.mockRestore(); }
  });
});
