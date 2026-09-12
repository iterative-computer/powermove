type LegacyPM = Record<string, any>;

export function showNewLayerMenu(PM: LegacyPM, anchor: HTMLElement): void {
  PM.menu(anchor, [
    { header: 'New layer' },
    { label: 'Text', kb: '⌘T', run: () => PM.cmd('newText') },
    { label: 'Shape', kb: '⌘⇧Y', run: () => PM.cmd('newShape') },
    { label: 'Solid', kb: '⌘Y', run: () => PM.cmd('newSolid') },
    { label: 'Null', run: () => PM.cmd('newNull') },
    '-',
    { label: 'Import media…', kb: '⌘I', run: () => PM.cmd('import') }
  ], { right: true });
}

export function showFxMenu(PM: LegacyPM, anchor: HTMLElement, selected?: any): void {
  const layer = selected ?? PM.firstSel?.();
  if (!layer || PM.TYPE_META?.[layer.type]?.effects === false) {
    PM.toast?.('Select a layer first');
    return;
  }

  const groups = new Map<string, Array<[string, any]>>();
  for (const [key, definition] of Object.entries(PM.FX ?? {}) as Array<[string, any]>) {
    const group = definition.group ?? 'Effects';
    const entries = groups.get(group) ?? [];
    entries.push([key, definition]);
    groups.set(group, entries);
  }

  const items: any[] = [];
  for (const [group, entries] of groups) {
    items.push({ header: group });
    for (const [key, definition] of entries) {
      items.push({
        label: definition.label,
        run: () => {
          PM.Edit.apply(
            { type: 'add_effect', target: layer.id, effect: key },
            { label: `Add ${definition.label}`, origin: 'inspector' }
          );
          PM.invalidate?.();
        }
      });
    }
  }
  PM.menu(anchor, items, { right: true });
}
