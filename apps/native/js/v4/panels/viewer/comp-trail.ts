/** Composition navigation retains descendants when stepping back up. */
export type CompTrail = { ids: string[]; index: number };
export function compTrail(
  previous: CompTrail | null,
  active: string | null | undefined,
  parents: (id: string) => string[],
  exists: (id: string) => boolean = () => true,
): CompTrail {
  if (!active) return { ids: [], index: -1 };
  const before: string[] = [];
  for (const id of previous?.ids ?? []) {
    if (
      !exists(id) ||
      (before.length && !parents(id).includes(before[before.length - 1]!))
    )
      break;
    before.push(id);
  }
  const at = before.indexOf(active);
  if (at >= 0) return { ids: before, index: at };
  const current =
    previous && previous.index >= 0 ? before[previous.index] : undefined;
  if (current && parents(active).includes(current)) {
    const ids = [...before.slice(0, before.indexOf(current) + 1), active];
    return { ids, index: ids.length - 1 };
  }
  const ids = [active],
    seen = new Set(ids);
  for (let guard = 0; guard < 32; guard++) {
    const up = parents(ids[0]!).filter((id) => !seen.has(id) && exists(id));
    if (!up.length) break;
    const next = up.find((id) => before.includes(id)) ?? up[0]!;
    ids.unshift(next);
    seen.add(next);
  }
  return { ids, index: ids.length - 1 };
}
