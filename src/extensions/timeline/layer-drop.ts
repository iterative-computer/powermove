/** Resolve pointer intent against visible rows, never the flat project index. */
export function layerDrop(rows: any[], rowPosition: number, x: number) {
  if (!rows.length) return null;
  let i = Math.max(0, Math.min(rows.length - 1, Math.floor(rowPosition)));
  while (i > 0 && rows[i].kind !== 'layer') i--;
  const row = rows[i], layer = row.L;
  const fraction = rowPosition - i;
  const inside = layer.type === 'group' && fraction >= .25 && fraction <= .75 && x >= 74 + Math.min(48, (row.depth || 0) * 12);
  const mode = inside ? 'inside' : fraction < .5 ? 'before' : 'after';
  let end = i + 1;
  while (end < rows.length && (rows[end].kind !== 'layer' || (rows[end].depth || 0) > (row.depth || 0))) end++;
  return { target: layer.id, group: inside ? layer.id : layer.group || null, mode, row: inside || mode === 'before' ? i : end,
    depth: (row.depth || 0) + (inside ? 1 : 0), name: layer.name };
}
