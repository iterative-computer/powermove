import type { Layer, MenuContribution, PowermoveAPI } from 'powermove';

export function showNewLayerMenu(api: PowermoveAPI, anchor: HTMLElement): void {
  api.ui.menu(anchor, [
    { header: 'New layer' },
    { label: 'Text', kb: '⌘T', run: () => api.commands.run('newText') },
    { label: 'Shape', kb: '⌘⇧Y', run: () => api.commands.run('newShape') },
    { label: 'Solid', kb: '⌘Y', run: () => api.commands.run('newSolid') },
    { label: 'Null', run: () => api.commands.run('newNull') },
    '-',
    { label: 'Import media…', kb: '⌘I', run: () => api.commands.run('import') }
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

  const items: MenuContribution[] = [];
  for (const [group, entries] of groups) {
    items.push({ header: group });
    for (const [key, definition] of entries) {
      if (!definition) continue;
      items.push({
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
  }
  api.ui.menu(anchor, items);
}
