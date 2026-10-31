import type { PMRegistry } from '../registry';
import { splitTextLayers, type TextSplitMode } from '../core/split-text';
import { evaluatedValue } from '../core/content-properties';
import { validMatteSource } from '../core/matte';

/** Apply the lower of two stack-selected layers as an alpha track matte for the upper layer. */
export function applySelectedLayerMatte(PM: PMRegistry, selected: any[], origin = 'timeline'): boolean {
  if (selected.length !== 2) return false;
  const layers = selected.slice().sort((a, b) => PM.proj.layers.indexOf(a) - PM.proj.layers.indexOf(b));
  const top = layers[0];
  const bottom = layers[1];
  if (!top || !bottom || top === bottom || ['audio', 'null', 'adjustment', 'group'].includes(bottom.type)) return false;
  if (top.lock || bottom.lock || (PM.groupAncestors?.(top) || []).some((group: any) => group.lock)
    || (PM.groupAncestors?.(bottom) || []).some((group: any) => group.lock)) return false;
  if (!validMatteSource(PM.proj.layers, top, bottom.id)) return false;
  PM.Edit.apply({ type: 'set_layer', target: top.id, patch: { matteSource: bottom.id } },
    { label: 'Use bottom layer as mask', origin });
  return true;
}

export function parentMenuItems(PM: PMRegistry, ids: string[], origin = 'timeline'): any[] {
  const selected = ids.map(id => PM.L(id)).filter((layer: any) => layer && PM.TYPE_META[layer.type]?.transform !== false);
  const apply = (parent: string | null) => PM.Edit.apply(selected.map((layer: any) => ({ type: 'set_layer', target: layer.id, patch: { parent } })), { label: parent ? 'Parent layers' : 'Remove parent', origin });
  return [
    { label: 'None', on: selected.every((layer: any) => !layer.parent), run: () => apply(null) },
    ...PM.proj.layers.filter((candidate: any) => PM.TYPE_META[candidate.type]?.transform !== false && selected.every((layer: any) => layer.id !== candidate.id && !PM.wouldCycle(layer, candidate.id)))
      .map((candidate: any) => ({ label: candidate.name, on: selected.every((layer: any) => layer.parent === candidate.id), run: () => apply(candidate.id) })),
  ];
}

/** Rename in the timeline row when it's on screen, otherwise in a small dialog. */
function renameLayerPrompt(PM: PMRegistry, layer: any, origin: string): void {
  if (PM.Kernel?.services?.get?.('timeline')?.renameLayer?.(layer)) return;
  const input = document.createElement('input');
  input.value = layer.name;
  input.setAttribute('aria-label', 'Layer name');
  const field = document.createElement('div');
  field.className = 'field';
  field.append(input);
  PM.modal?.({ title: 'Rename Layer', body: field, width: 400, actions: [
    { label: 'Cancel' },
    { label: 'Rename', pri: true, run: () => {
      const name = input.value.trim();
      if (name && name !== layer.name) PM.Edit.apply({ type: 'set_layer', target: layer.id, patch: { name } }, { label: 'Rename layer', origin });
    } },
  ] });
  window.setTimeout(() => { input.focus(); input.select(); }, 30);
}

export function installLayerMenu(PM: PMRegistry): void {
  PM.showParentMenu = (ids: string[], event: {clientX: number; clientY: number}) => PM.menu(document.body, [{header:'Link Layers'}, ...parentMenuItems(PM, ids)], {x:event.clientX,y:event.clientY});
  PM.showLayerMenu = (layer: any, event: { clientX: number; clientY: number }, origin = 'timeline') => {
    if (!PM.sel.layers.includes(layer.id)) PM.selectLayers(layer.id);
    const selected = PM.selLayers();
    const lockedGroup = (PM.groupAncestors?.(layer) || []).filter((group: any) => group.lock).at(-1);
    const editable = selected.every((item: any) => !item.lock && !(PM.groupAncestors?.(item) || []).some((group: any) => group.lock));
    const editOrigin = origin === 'viewer' ? 'canvas' : origin;
    const apply = (commands: any, label: string) => PM.Edit.apply(commands, { label, origin: editOrigin });
    const patch = (value: any, label: string) => apply(selected.map((item: any) => ({ type: 'set_layer', target: item.id, patch: value })), label);
    const open = (items: any[] | Promise<any[]>) => PM.menu(document.body, items, { x: event.clientX, y: event.clientY });
    const toggleAll = (key: string) => selected.every((item: any) => item[key]);
    /* The most-used edits first, then where the layer sits, then its switches,
       and Delete last on its own. A title only for a multiple selection, to
       say how many layers the commands act on. */
    const items: any[] = [];
    if (selected.length > 1) items.push({ header: `${selected.length} Layers` });
    if (selected.length === 1) items.push({ label: 'Rename…', disabled: !editable, run: () => renameLayerPrompt(PM, layer, editOrigin) });
    items.push(
      { label: 'Duplicate', icon: 'copy', kb: '⌘D', disabled: !editable, run: () => PM.cmd('duplicate') },
      { label: 'Group Layers', icon: 'stack', kb: '⌘G', disabled: !editable, run: () => PM.cmd('groupLayers') },
    );
    if (selected.some((item: any) => item.type === 'group')) items.push({ label: 'Ungroup Layers', icon: 'layers', kb: '⇧⌘G', disabled: !editable, run: () => PM.cmd('ungroupLayers') });
    items.push({ label: 'Pre-compose…', icon: 'frame', kb: '⇧⌘C', disabled: !editable, run: () => PM.cmd('precompose') });
    /* Un-precompose: the pre-comp becomes a group of its layers. */
    if (selected.length === 1 && layer.type === 'precomp') items.push({ label: 'Release Pre-comp', icon: 'layers', disabled: !editable, run: () => PM.cmd('releasePrecomp', layer.id) });
    if (selected.every((item: any) => item.type === 'text')) items.push({ label: 'Split Text into Layers', icon: 'type', disabled: !editable, submenu:
      ([['By Word', 'words'], ['By Character', 'characters'], ['By Line', 'lines']] as const).map(([label, mode]) => ({ label, run: () => splitTextLayers(PM, selected.map((item: any) => item.id), mode as TextSplitMode) })) });
    if (selected.length === 2) {
      const matteLayers = selected.slice().sort((a: any, b: any) => PM.proj.layers.indexOf(a) - PM.proj.layers.indexOf(b));
      const canMatte = matteLayers[0] && matteLayers[1]
        && !['audio', 'null', 'adjustment', 'group'].includes(matteLayers[1].type)
        && validMatteSource(PM.proj.layers, matteLayers[0], matteLayers[1].id);
      items.push({ label: 'Use Bottom Layer as Mask', icon: 'mask', disabled: !editable || !canMatte,
        run: () => applySelectedLayerMatte(PM, selected, editOrigin) });
    }

    items.push('-');
    const groups = PM.proj.layers.filter((item: any) => item.type === 'group' && !item.lock && !PM.expandGroups(PM.sel.layers).includes(item.id));
    if (groups.length || selected.some((item: any) => item.group)) items.push({ label: 'Move to Group', icon: 'enter', disabled: !editable, submenu: [
      { label: 'Outside Groups', run: () => apply({ type: 'move_to_group', targets: PM.sel.layers, group: null }, 'Move out of group') },
      ...groups.map((group: any) => ({ label: group.name, run: () => apply({ type: 'move_to_group', targets: PM.sel.layers, group: group.id }, 'Move to group') }))
    ] });
    if (selected.every((item: any) => PM.TYPE_META[item.type]?.transform !== false)) items.push({ label: 'Link Layers', icon: 'link', disabled: !editable, submenu: parentMenuItems(PM, selected.map((item: any) => item.id), editOrigin) });
    if (selected.every((item: any) => item.type !== 'group')) {
      const inside = selected.every((item: any) => PM.time > item.from && PM.time < item.from + item.dur);
      items.push({ label: 'Timing', icon: 'clock', disabled: !editable, submenu: [
        { label: 'Split at Playhead', kb: '⇧⌘D', disabled: !inside, run: () => PM.cmd('split') },
        { label: 'Trim In to Playhead', disabled: !inside, run: () => apply(selected.flatMap((item: any) => [
          { type: 'set_layer', target: item.id, patch: { from: PM.time, duration: item.dur - (PM.time - item.from) } },
          ...((patch => patch ? [{ type: 'set_content', target: item.id, patch }] : [])(PM.MediaTiming.startPatch(item, PM.time)))
        ]), 'Trim in') },
        { label: 'Trim Out to Playhead', disabled: !inside, run: () => apply(selected.map((item: any) => ({ type: 'set_layer', target: item.id, patch: { duration: PM.time - item.from } })), 'Trim out') },
        { label: 'Fit to Composition', run: () => patch({ from: 0, duration: PM.proj.dur }, 'Fit duration') },
      ] });
    }
    if (selected.length === 1 && layer.type === 'video' && layer.d?.embeddedAudio === true) {
      const muted = layer.d.audioMuted === true;
      items.push({ label: muted ? 'Unmute Audio' : 'Mute Audio', icon: muted ? 'speaker' : 'speakerOff', disabled: !editable, run: () => apply({ type: 'set_content', target: layer.id, patch: { audioMuted: !muted } }, muted ? 'Unmute audio' : 'Mute audio') });
      items.push({ label: 'Separate Audio', icon: 'waveform', disabled: !editable, run: () => PM.cmd('separateAudio', layer.id) });
    }

    const visible = evaluatedValue(PM, layer, layer.on, PM.time, 'l.on');
    items.push('-',
      { label: visible ? 'Hide' : 'Show', icon: visible ? 'eyeoff' : 'eye', disabled: !editable, run: () => patch({ visible: !visible }, 'Visibility') },
      { label: layer.solo ? 'Unsolo' : 'Solo', icon: 'headphones', disabled: !editable, run: () => patch({ solo: !layer.solo }, 'Solo layers') },
      { label: lockedGroup ? 'Unlock Group' : layer.lock ? 'Unlock' : 'Lock', icon: 'lock', run: () => lockedGroup ? apply({type:'set_layer',target:lockedGroup.id,patch:{locked:false}}, 'Unlock group') : patch({ locked: !layer.lock }, 'Lock layers') });
    if (selected.every((item: any) => item.type !== 'audio' && item.type !== 'adjustment')) items.push({ label: '3D Layer', icon: 'cube', on: toggleAll('threeD'), disabled: !editable, run: () => patch({threeD: !toggleAll('threeD')}, '3D layer') });
    if (selected.every((item: any) => item.type === 'precomp')) items.push({ label: 'Continuously Rasterize', icon: 'sun', on: toggleAll('continuousRaster'), disabled: !editable,
      run: () => patch({ continuousRaster: !toggleAll('continuousRaster') }, 'Continuously rasterize') });
    items.push('-', { label: 'Delete', icon: 'trash', kb: '⌫', disabled: !editable, run: () => PM.cmd('delete') });
    /* Each open asks its contributors afresh, with this layer. Sandboxed ones
       answer over a port, within the kernel's deadline. */
    const contributions = [PM.Kernel?.gatherMenu?.('layer:context', { layerId: layer.id }) || [],
      PM.Kernel?.gatherMenu?.(`${origin}:context`, { kind: 'layer', layerId: layer.id, time: PM.time }) || []];
    /* Contributed rows form their own group, above Delete so it stays last. */
    const withContributions = (lists: any[][]) => {
      const contributed = lists.flat();
      return contributed.length ? [...items.slice(0, -2), '-', ...contributed, ...items.slice(-2)] : items;
    };
    open(contributions.every(Array.isArray) ? withContributions(contributions) : Promise.all(contributions).then(withContributions));
  };
}
