/** Bounded postorder Merkle work sent to the host. Numeric references name
 * an earlier instruction in this batch; strings are already resolved hashes. */
export type DigestInstruction =
  | { value: any; prefix?: string }
  | { array: Array<string | number> }
  | { object: Array<[string, string | number]> };

const hex = /^[0-9a-f]{64}$/;

/** Host-side pure processor. The host supplies its independent SHA/canonical
 * implementation; this shared module contains no Node or browser SHA code. */
export function digestInstructions(instructions: readonly DigestInstruction[], hash: (value: any, prefix?: string) => string): string[] {
  if (!Array.isArray(instructions)) throw new TypeError('Invalid digest instructions');
  const hashes: string[] = [];
  const reference = (ref: unknown, index: number): string => {
    if (typeof ref === 'string' && hex.test(ref)) return ref;
    if (typeof ref === 'number' && Number.isSafeInteger(ref) && ref >= 0 && ref < index) return hashes[ref]!;
    throw new TypeError('A digest reference must be a hash or an earlier instruction');
  };
  for (let index = 0; index < instructions.length; index++) {
    const instruction = instructions[index];
    if (!instruction || typeof instruction !== 'object') throw new TypeError('Invalid digest instruction');
    const kinds = ['value', 'array', 'object'].filter(key => Object.hasOwn(instruction, key));
    if (kinds.length !== 1) throw new TypeError('Invalid digest instruction kind');
    let value: any, prefix: string | undefined;
    if (kinds[0] === 'value') {
      const item = instruction as { value: any; prefix?: string };
      if (item.prefix !== undefined && typeof item.prefix !== 'string') throw new TypeError('Invalid digest prefix');
      value = item.value;
      prefix = item.prefix;
    } else if (kinds[0] === 'array') {
      const item = instruction as { array: Array<string | number> };
      if (!Array.isArray(item.array)) throw new TypeError('Invalid digest array');
      value = ['a', item.array.map(ref => reference(ref, index))];
    } else {
      const item = instruction as { object: Array<[string, string | number]> };
      if (!Array.isArray(item.object)) throw new TypeError('Invalid digest object');
      value = ['o', item.object.map(member => {
        if (!Array.isArray(member) || member.length !== 2 || typeof member[0] !== 'string') throw new TypeError('Invalid digest member');
        return [member[0], reference(member[1], index)];
      })];
    }
    const digest = hash(value, prefix);
    if (typeof digest !== 'string' || !hex.test(digest)) throw new TypeError('The digest processor returned an invalid SHA-256');
    hashes.push(digest);
  }
  return hashes;
}
