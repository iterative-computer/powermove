import type { PMRegistry } from '../registry';
import { splitTextLayers, type TextSplitMode } from '../core/split-text';
import { evaluatedValue } from '../core/content-properties';

export function parentMenuItems(PM: PMRegistry, ids: string[], origin = 'timeline'): any[] {
  const selected = ids.map(id => PM.L(id)).filter((layer: any) => layer && PM.TYPE_META[layer.type]?.transform !== false);
  const apply = (parent: string | null) => PM.Edit.apply(selected.map((layer: any) => ({ type: 'set_layer', target: layer.id, patch: { parent } })), { label: parent ? 'Parent layers' : 'Remove parent', origin });
  return [
    { label: 'None', on: selected.every((layer: any) => !layer.parent), run: () => apply(null) },
    ...PM.proj.layers.filter((candidate: any) => PM.TYPE_META[candidate.type]?.transform !== false && selected.every((layer: any) => layer.id !== candidate.id && !PM.wouldCycle(layer, candidate.id)))
      .map((candidate: any) => ({ label: candidate.name, on: selected.every((layer: any) => layer.parent === candidate.id), run: () => apply(candidate.id) })),
  ];
}

export function installLayerMenu(PM: PMRegistry): void {
  PM.showParentMenu = (ids: string[], event: {clientX: number; clientY: number}) => PM.menu(document.body, [{header:'Parent'}, ...parentMenuItems(PM, ids)], {x:event.clientX,y:event.clientY});
  PM.showLayerMenu = (layer: any, event: { clientX: number; clientY: number }, origin = 'timeline') => {
    if (!PM.sel.layers.includes(layer.id)) PM.selectLayers(layer.id);
    const selected = PM.selLayers();
    const lockedGroup = (PM.groupAncestors?.(layer) || []).filter((group: any) => group.lock).at(-1);
    const editable = selected.every((item: any) => !item.lock && !(PM.groupAncestors?.(item) || []).some((group: any) => group.lock));
    const editOrigin = origin === 'viewer' ? 'canvas' : origin;
    const apply = (commands: any, label: string) => PM.Edit.apply(commands, { label, origin: editOrigin });
    const patch = (value: any, label: string) => apply(selected.map((item: any) => ({ type: 'set_layer', target: item.id, patch: value })), label);
    const open = (items: any[]) => PM.menu(document.body, items, { x: event.clientX, y: event.clientY });
    const more = (title: string, items: any[]) => open([{ label: '‹ Back', run: () => PM.showLayerMenu(layer, event, origin) }, { header: title }, ...items]);
    const items: any[] = [
      { header: selected.length > 1 ? `${selected.length} layers` : layer.name },
      { label: 'Duplicate', kb: '⌘D', disabled: !editable, run: () => PM.cmd('duplicate') },
      { label: 'Group layers', kb: '⌘G', disabled: !editable, run: () => PM.cmd('groupLayers') },
    ];
    if (selected.every((item: any) => item.type === 'text')) items.push({ label: 'Split text into layers…', disabled: !editable, run: () => more('Split text into layers',
      ([['By word', 'words'], ['By character', 'characters'], ['By line', 'lines']] as const).map(([label, mode]) => ({ label, run: () => splitTextLayers(PM, selected.map((item: any) => item.id), mode as TextSplitMode) }))) });
    if (selected.every((item: any) => item.type !== 'audio' && item.type !== 'adjustment')) items.push({ label: '3D layer', on: selected.every((item: any) => item.threeD), disabled: !editable, run: () => patch({threeD: !selected.every((item: any) => item.threeD)}, '3D layer') });
    if (selected.some((item: any) => item.type === 'group')) items.push({ label: 'Ungroup layers', kb: '⌘⇧G', disabled: !editable, run: () => PM.cmd('ungroupLayers') });
    const groups = PM.proj.layers.filter((item: any) => item.type === 'group' && !item.lock && !PM.expandGroups(PM.sel.layers).includes(item.id));
    if (groups.length || selected.some((item: any) => item.group)) items.push({ label: 'Move to group…', disabled: !editable, run: () => more('Move to group', [
      { label: 'Outside groups', run: () => apply({ type: 'move_to_group', targets: PM.sel.layers, group: null }, 'Move out of group') },
      ...groups.map((group: any) => ({ label: group.name, run: () => apply({ type: 'move_to_group', targets: PM.sel.layers, group: group.id }, 'Move to group') }))
    ]) });
    if (selected.every((item: any) => PM.TYPE_META[item.type]?.transform !== false)) items.push({ label: 'Parent…', disabled: !editable, run: () => more('Parent', parentMenuItems(PM, selected.map((item: any) => item.id), editOrigin)) });
    if (selected.every((item: any) => item.type !== 'group')) {
      const inside = selected.every((item: any) => PM.time > item.from && PM.time < item.from + item.dur);
      items.push({ label: 'Timing…', disabled: !editable, run: () => more('Timing', [
        { label: 'Split at playhead', kb: '⌘⇧D', disabled: !inside, run: () => PM.cmd('split') },
        { label: 'Trim in to playhead', disabled: !inside, run: () => apply(selected.flatMap((item: any) => [
          { type: 'set_layer', target: item.id, patch: { from: PM.time, duration: item.dur - (PM.time - item.from) } },
          ...(PM.MediaTiming.isTimed(item) ? [{ type: 'set_content', target: item.id, patch: { trim: PM.MediaTiming.trimAtStart(item, PM.time) } }] : [])
        ]), 'Trim in') },
        { label: 'Trim out to playhead', disabled: !inside, run: () => apply(selected.map((item: any) => ({ type: 'set_layer', target: item.id, patch: { duration: PM.time - item.from } })), 'Trim out') },
        { label: 'Fit to composition', run: () => patch({ from: 0, duration: PM.proj.dur }, 'Fit duration') },
      ]) });
    }
    if (selected.length === 1 && layer.type === 'video' && layer.d?.embeddedAudio === true) items.push({ label: 'Separate audio', disabled: !editable, run: () => PM.cmd('separateAudio', layer.id) });
    items.push('-',
      { label: evaluatedValue(PM, layer, layer.on, PM.time, 'l.on') ? 'Hide' : 'Show', disabled: !editable, run: () => patch({ visible: !evaluatedValue(PM, layer, layer.on, PM.time, 'l.on') }, 'Visibility') },
      { label: layer.solo ? 'Unsolo' : 'Solo', disabled: !editable, run: () => patch({ solo: !layer.solo }, 'Solo layers') },
      { label: lockedGroup ? 'Unlock group' : layer.lock ? 'Unlock' : 'Lock', run: () => lockedGroup ? apply({type:'set_layer',target:lockedGroup.id,patch:{locked:false}}, 'Unlock group') : patch({ locked: !layer.lock }, 'Lock layers') },
      '-', { label: 'Delete', kb: '⌫', disabled: !editable, run: () => PM.cmd('delete') });
    const contributed = [...(PM.Kernel?.collectMenu?.('layer:context', { layerId: layer.id }) || []),
      ...(PM.Kernel?.collectMenu?.(`${origin}:context`, { kind: 'layer', layerId: layer.id, time: PM.time }) || [])];
    if (contributed.length) items.push('-', ...contributed);
    open(items);
  };
}
