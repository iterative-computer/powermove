import type { PMRegistry } from '../registry';
import { finite } from './project-tools';
import { isProperty, resolveContent } from '../core/content-properties';

function targets(PM: PMRegistry, ids: unknown): any[] {
  if (!Array.isArray(ids) || !ids.length || ids.length > 500 || ids.some(id => typeof id !== 'string') || new Set(ids).size !== ids.length) throw new Error('Supply 1–500 unique layerIds from get_timeline.');
  return ids.map(id => {
    const layer = PM.proj.layers.find((item: any) => item.id === id);
    if (!layer) throw new Error(`Layer not found: ${id}`);
    return layer;
  });
}
function editable(PM: PMRegistry, layers: any[]) {
  for (const layer of layers) {
    if (layer.lock || (PM.groupAncestors?.(layer) || []).some((group: any) => group.lock)) throw new Error(`Layer or enclosing group is locked: ${layer.name}`);
    if (Object.values(layer.locked_intent || {}).some(Boolean)) throw new Error(`Layer has protected hand edits: ${layer.name}`);
  }
}
function frame(PM: PMRegistry, value: unknown, name: string, negative = false): number {
  const number = finite(value, name, negative ? -PM.proj.dur : 0, PM.proj.dur);
  return Math.round(number * PM.proj.fps) / PM.proj.fps;
}
function fit(PM: PMRegistry, layer: any, from: number) {
  if (from < -1e-7 || from + layer.dur > PM.proj.dur + 1e-7) throw new Error('Every affected strip must fit the composition. Extend it with apply_commands first.');
}

/** Compact stack/timing information, independent of property and keyframe pages. */
export function timelineState(PM: PMRegistry, args: Record<string, any>) {
  const offset = args.offset === undefined ? 0 : finite(args.offset, 'offset', 0, Number.MAX_SAFE_INTEGER);
  const limit = args.limit === undefined ? 100 : finite(args.limit, 'limit', 1, 500);
  if (!Number.isInteger(offset) || !Number.isInteger(limit)) throw new Error('offset and limit must be integers.');
  const layers = PM.proj.layers;
  return {
    projectId: PM.proj.id, revision: PM.proj.revision || 0, fps: PM.proj.fps, duration: PM.proj.dur,
    playhead: PM.time, workArea: PM.proj.work, selection: PM.sel.layers,
    total: layers.length, offset, nextOffset: offset + limit < layers.length ? offset + limit : null,
    stackOrder: 'Topmost first. Each layer has its own strip; overlaps are composited in stack order.',
    strips: layers.slice(offset, offset + limit).map((layer: any, n: number) => {
      const media = ['video', 'audio'].includes(layer.type);
      const d = media ? resolveContent(PM, layer, layer.from) : {};
      const animatedTiming = media && ['trim', 'speed', 'sourceTime', 'timeRemap'].some(key =>
        isProperty(layer.d[key]) && (layer.d[key].kf.length || layer.d[key].expr) || key === 'timeRemap' && !!d[key]);
      return { id: layer.id, index: offset + n, name: layer.name, type: layer.type,
        from: layer.from, end: layer.from + layer.dur, duration: layer.dur,
        startFrame: Math.round(layer.from * PM.proj.fps), endFrame: Math.round((layer.from + layer.dur) * PM.proj.fps),
        group: layer.group || null, parent: layer.parent || null, locked: !!layer.lock,
        protected: Object.keys(layer.locked_intent || {}).filter(key => layer.locked_intent[key]),
        visible: isProperty(layer.on) ? PM.evP(layer, layer.on, PM.time, 'l.on') : layer.on,
        solo: !!layer.solo, shy: !!layer.shy, color: layer.color,
        ...(media ? { assetId: d.asset, sourceIn: d.trim ?? 0, speed: d.speed ?? 1, animatedTiming: !!animatedTiming,
          sourceOut: animatedTiming ? null : Number(d.trim || 0) + layer.dur * Number(d.speed ?? 1), embeddedAudio: d.embeddedAudio === true } : {}),
      };
    }),
  };
}

/** Batch gestures are atomic and use the same history group as the other live tools. */
export function editLayerStrips(PM: PMRegistry, args: Record<string, any>, meta: Record<string, any>) {
  const selected = targets(PM, args.layerIds);
  const operation = args.operation;
  if (!['move', 'sequence', 'align', 'duplicate', 'reorder', 'state', 'remove', 'close_gap'].includes(operation)) throw new Error('Unknown strip operation.');
  if (operation === 'state') {
    const patch = args.patch;
    if (!patch || typeof patch !== 'object' || Array.isArray(patch) || !Object.keys(patch).length) throw new Error('Supply a state patch.');
    for (const [key, value] of Object.entries(patch)) {
      if (!['name', 'color', 'visible', 'solo', 'shy', 'locked', 'collapsed'].includes(key)) throw new Error(`Unsupported strip state: ${key}`);
      if (['name', 'color'].includes(key) ? typeof value !== 'string' || !value.trim() : typeof value !== 'boolean') throw new Error(`Invalid ${key}.`);
    }
    editable(PM, selected);
    return PM.Edit.apply(selected.map(layer => ({ type: 'set_layer', target: layer.id, patch })), meta);
  }
  const expanded = PM.expandGroups?.(selected.map(layer => layer.id)) || selected.map(layer => layer.id);
  const layers = PM.proj.layers.filter((layer: any) => expanded.includes(layer.id));
  editable(PM, layers);
  return PM.Edit.mutate(meta.label, () => {
    let result: any = { layerIds: layers.map((layer: any) => layer.id) };
    if (operation === 'move' || operation === 'duplicate') {
      const delta = args.at === undefined ? frame(PM, args.delta, 'delta', true) : frame(PM, args.at, 'at') - Math.min(...layers.map((layer: any) => layer.from));
      for (const layer of layers) fit(PM, layer, layer.from + delta);
      if (operation === 'move') for (const layer of layers) layer.from += delta;
      else {
        const copies = layers.map((layer: any) => PM.cloneLayer(layer));
        const ids = new Map(layers.map((layer: any, i: number) => [layer.id, copies[i].id]));
        for (const copy of copies) {
          copy.from += delta;
          for (const key of ['parent', 'group', 'matteSource']) if (ids.has(copy[key])) copy[key] = ids.get(copy[key]);
        }
        const roots = selected.filter(layer => !(PM.groupAncestors?.(layer) || []).some((group: any) => selected.some(item => item.id === group.id)));
        const bySource = new Map(layers.map((layer: any, i: number) => [layer.id, copies[i]]));
        PM.proj.layers = PM.proj.layers.flatMap((layer: any) => {
          if (!roots.some(root => root.id === layer.id)) return [layer];
          const block = PM.expandGroups?.([layer.id]) || [layer.id];
          return [...layers.filter((item: any) => block.includes(item.id)).map((item: any) => bySource.get(item.id)), layer];
        });
        result = { layerIds: copies.map((layer: any) => layer.id), copies: layers.map((layer: any, i: number) => ({ sourceId: layer.id, layerId: copies[i].id })) };
      }
    } else if (operation === 'sequence' || operation === 'align') {
      if (selected.some(layer => layer.type === 'group')) throw new Error('Sequence or align individual strips; move a group as a block.');
      let at = frame(PM, args.at, 'at');
      const gap = operation === 'sequence' ? frame(PM, args.gap ?? 0, 'gap', true) : 0;
      if (operation === 'align' && !['start', 'end'].includes(args.edge ?? 'start')) throw new Error('Align edge must be start or end.');
      // Explicit input order is the sequence order, not the visual stack order.
      for (const layer of selected) {
        const from = operation === 'align' && args.edge === 'end' ? at - layer.dur : at;
        fit(PM, layer, from); layer.from = from;
        if (operation === 'sequence') at = Math.round((at + layer.dur + gap) * PM.proj.fps) / PM.proj.fps;
      }
    } else if (operation === 'reorder') {
      const anchor = targets(PM, [args.anchorId])[0];
      editable(PM, [anchor]);
      if (expanded.includes(anchor.id)) throw new Error('The anchor must be outside the moved strips.');
      if (!['before', 'after'].includes(args.placement)) throw new Error('Choose before or after.');
      if (selected.some(layer => (layer.group || null) !== (anchor.group || null))) throw new Error('Reorder siblings; use move_to_group to change membership.');
      const remaining = PM.proj.layers.filter((layer: any) => !expanded.includes(layer.id));
      const anchorBlock = PM.expandGroups?.([anchor.id]) || [anchor.id];
      const index = args.placement === 'after' ? Math.max(...remaining.map((layer: any, i: number) => anchorBlock.includes(layer.id) ? i : -1)) + 1 : remaining.indexOf(anchor);
      remaining.splice(index, 0, ...layers); PM.proj.layers = remaining;
    } else if (operation === 'close_gap') {
      const start = frame(PM, args.start, 'start'), end = frame(PM, args.end, 'end');
      if (end <= start) throw new Error('Gap must be at least one frame.');
      if (layers.some((layer: any) => layer.from < end - 1e-7 && layer.from + layer.dur > start + 1e-7)) throw new Error('The gap intersects an affected strip. Trim, split, or remove it first.');
      for (const layer of layers) if (layer.from >= end - 1e-7) layer.from -= end - start;
    } else if (operation === 'remove') {
      const affected = PM.proj.layers.filter((layer: any) => !expanded.includes(layer.id) && expanded.includes(layer.parent));
      editable(PM, affected);
      PM.removeLayers(expanded);
    }
    PM.ProjectIndex?.invalidate();
    return result;
  }, meta);
}
