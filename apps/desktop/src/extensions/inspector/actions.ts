import type { Layer, MenuContribution, PowermoveAPI } from 'powermove';

export function showNewLayerMenu(api: PowermoveAPI, anchor: HTMLElement): void {
  api.ui.menu(anchor, [
    /* Same rows, order and names as New ▸ and Layer ▸ New. */
    { header: 'New Layer' },
    { label: 'Text', kb: '⌘T', run: () => api.commands.run('newText') },
    { label: 'Solid', kb: '⌘Y', run: () => api.commands.run('newSolid') },
    { label: 'Shape Layer', kb: '⌘⇧Y', run: () => api.commands.run('newShape') },
    { label: 'Null Object', kb: '⌘⌥⇧Y', run: () => api.commands.run('newNull') },
    { label: 'Adjustment Layer', kb: '⌘⌥Y', run: () => api.commands.run('newAdjustment') },
    '-',
    { label: 'Import Media…', kb: '⌘I', run: () => api.commands.run('import') }
  ]);
}

export function showFxMenu(api: PowermoveAPI, anchor: HTMLElement, selected?: Layer | null): void {
  const layer = selected ?? api.selection.first();
  const metadata = layer ? api.model.TYPE_META[layer.type] : null;
  if (!layer || (metadata && 'effects' in metadata && metadata.effects === false)) {
    api.ui.toast('Select a layer first');
    return;
  }

  const groups = new Map<string, Array<[string, ReturnType<PowermoveAPI['effects']['get']>]>>();
  for (const definition of api.effects.list()) {
    const group = definition.group ?? 'Effects';
    const entries = groups.get(group) ?? [];
    entries.push([definition.id, definition]);
    groups.set(group, entries);
  }

  /* One submenu per category, as Effect ▸ in After Effects, rather than one
     long list. */
  const items: MenuContribution[] = [];
  for (const [group, entries] of groups) {
    const submenu: MenuContribution[] = [];
    for (const [key, definition] of entries) {
      if (!definition) continue;
      submenu.push({
        label: definition.label,
        run: () => {
          api.edit.apply(
            { type: 'add_effect', target: layer.id, effect: key },
            { label: `Add ${definition.label}`, origin: 'inspector' }
          );
          api.transport.invalidate();
        }
      });
    }
    if (submenu.length) items.push({ label: group, submenu });
  }
  api.ui.menu(anchor, items);
}
