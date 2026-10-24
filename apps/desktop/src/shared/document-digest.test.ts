import { createHash } from 'node:crypto';
import { describe, expect, it } from 'vitest';
import { canonicalStringify } from './canonical-json';
import { digestInstructions, type DigestInstruction } from './document-digest';

const hash = (value: any, prefix = '') => createHash('sha256').update(prefix).update(canonicalStringify(value)!).digest('hex');

describe('digestInstructions', () => {
  it('resolves postorder references and prior-batch hashes exactly', () => {
    const scalar = hash(3, 'v');
    const instructions: DigestInstruction[] = [{ value: 3, prefix: 'v' }, { array: [0, scalar] }, { object: [['a', 1], ['z', scalar]] }];
    const result = digestInstructions(instructions, hash);
    expect(result).toEqual([scalar, hash(['a', [scalar, scalar]]), hash(['o', [['a', hash(['a', [scalar, scalar]])], ['z', scalar]]])]);
  });

  it('rejects self, forward, invalid numeric and malformed hash references', () => {
    for (const ref of [0, 1, -1, 0.5, Infinity, NaN, 'invalid', 'A'.repeat(64)]) {
      expect(() => digestInstructions([{ array: [ref] }], hash)).toThrow(/reference/);
    }
    expect(() => digestInstructions([{ value: 1 }, { object: [['x', 2]] }], hash)).toThrow(/reference/);
    expect(() => digestInstructions([{ value: 1, array: [] } as any], hash)).toThrow(/kind/);
    expect(() => digestInstructions([{ object: [[3, '0'.repeat(64)]] } as any], hash)).toThrow(/member/);
    expect(() => digestInstructions([{ value: 1 }], () => 'bad')).toThrow(/SHA-256/);
  });
});
