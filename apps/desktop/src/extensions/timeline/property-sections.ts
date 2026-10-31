/* An opened layer lists its keyframed properties under the same headings, in
   the same order, as the Properties panel: Content, Transform, parameters,
   each effect, masks, then layer options. */
export interface PropertySection { id: string; label: string; rank: number }

export function propertySection(layer: any, row: { key: string; group?: string }, effectLabel: (fx: any) => string): PropertySection {
  const key = String(row.key);
  if (key.startsWith('c.')) return { id: 'content', label: 'Content', rank: 0 };
  if (row.group === 'Transform' || key === 'scale' || key === 'position') return { id: 'transform', label: 'Transform', rank: 1 };
  if (key.startsWith('u.') || key.startsWith('x.')) return { id: 'parameters', label: row.group || 'Parameters', rank: 2 };
  const effects: any[] = Array.isArray(layer?.fx) ? layer.fx : [];
  const index = effects.findIndex((fx: any) => key.startsWith(fx.id + '.'));
  if (index >= 0) return { id: 'fx:' + effects[index].id, label: effectLabel(effects[index]), rank: 3 + index / 1000 };
  if (key.startsWith('m.')) return { id: 'mask:' + (row.group || ''), label: row.group || 'Mask', rank: 4 };
  if (key.startsWith('l.')) return { id: 'layer', label: 'Layer', rank: 6 };
  return { id: 'other:' + (row.group || ''), label: row.group || 'Other', rank: 5 };
}

/** Timeline rows for an opened layer: a heading before each section's properties. */
export function sectionedPropertyRows(layer: any, props: any[], effectLabel: (fx: any) => string): any[] {
  const entries = props.map((prop, index) => ({ prop, index, section: propertySection(layer, prop, effectLabel) }));
  entries.sort((a, b) => a.section.rank - b.section.rank || a.index - b.index);
  const rows: any[] = [];
  let current: string | null = null;
  for (const { prop, section } of entries) {
    if (section.id !== current) {
      rows.push({ kind: 'heading', L: layer, key: 'section:' + section.id, label: section.label });
      current = section.id;
    }
    rows.push({ kind: 'prop', L: layer, ...prop });
  }
  return rows;
}
