/** A stable new id for a repeated one: `base.2`, `base.3`, … the first not in `taken`.
 *  Opening the same file twice gives the same document, where a random id did not. */
export function derivedId(base: string, taken: { has(id: string): boolean }): string {
  for (let n = 2; ; n++) {
    const id = `${base}.${n}`;
    if (!taken.has(id)) return id;
  }
}
