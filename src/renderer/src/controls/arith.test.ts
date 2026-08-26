import { describe, expect, it } from 'vitest';
import { parseArithmetic } from './arith';

describe('parseArithmetic', () => {
  it.each([
    ['1 + 2 * 3', 7],
    ['(1 + 2) * 3', 9],
    ['-4 / 2 + +3', 1],
    ['.5 * 8', 4],
    ['1e2 + 2.5e-1', 100.25],
    ['--2', 2],
    [' 42 ', 42]
  ])('evaluates %s', (source, expected) => {
    expect(parseArithmetic(source)).toBe(expected);
  });

  it.each([
    '', '1+', '(1+2', '1 2', '2**3', '1/0', 'NaN', 'Infinity',
    'globalThis.alert(1)', 'constructor.constructor("return 1")()',
    '1; globalThis.pwned = true', '${globalThis.process}'
  ])('rejects invalid or executable input: %s', (source) => {
    expect(parseArithmetic(source)).toBeNaN();
  });
});

