/* Ported from js/core/selection.js — behavior-preserving. */
import type { PMRegistry } from '../registry';

export function install(PM: PMRegistry): void {
PM.projGeneration = Number.isSafeInteger(PM.projGeneration) ? PM.projGeneration : 0;

function keyframes(project: any) {
  const out: any[] = [];
  const projects = [project];
  const seen = new Set<any>();
  while (projects.length) {
    const comp: any = projects.shift();
    if (!comp || seen.has(comp)) continue;
    seen.add(comp);
    for (const layer of comp.layers || []) {
      const props = [...Object.values(layer.p || {})] as any[];
      for (const effect of layer.fx || []) props.push(...Object.values(effect.p || {}));
      for (const mask of layer.masks || []) props.push(...Object.values(mask.p || {}));
      if (layer.type === 'shader') props.push(...Object.values(layer.d?.uniforms || {}));
      for (const field of ['transitionIn', 'transitionOut']) props.push(...Object.values(layer[field]?.p || {}));
      for (const prop of props) for (const key of prop?.kf || []) if (typeof key?.i === 'string') out.push(key);
    }
    projects.push(...Object.values(comp.comps || {}) as any[]);
  }
  return out;
}

function stringIds(values: any) {
  const seen = new Set<any>();
  return (Array.isArray(values) ? values : []).filter((id: any) => {
    if (typeof id !== 'string' || !id || seen.has(id)) return false;
    seen.add(id);
    return true;
  });
}

PM.resolveSelectedKeys = () => {
  const ids = stringIds(PM.sel?.keys);
  if (!ids.length || !PM.proj) return [];
  const byId = new Map(keyframes(PM.proj).map(key => [key.i, key]));
  return ids.map((id: any) => byId.get(id)).filter(Boolean);
};

PM.replaceProject = (next: any, opts: any = {}) => {
  const selection = opts.selection || PM.sel || {};
  PM.proj = next;
  PM.projGeneration++;

  const layerIds = new Set((next?.layers || []).map((layer: any) => layer.id));
  PM.sel.layers = stringIds(selection.layers).filter((id: any) => layerIds.has(id));
  PM.sel.keys = stringIds(selection.keys);
  const liveKeyIds = new Set(keyframes(next).map(key => key.i));
  PM.sel.keys = PM.sel.keys.filter((id: any) => liveKeyIds.has(id));
  if (opts.selection) PM.sel.chan = typeof selection.chan === 'string' ? selection.chan : null;

  const duration = Number.isFinite(Number(next?.dur)) ? Math.max(0, Number(next.dur)) : 0;
  const time = Number(PM.time);
  PM.time = Math.max(0, Math.min(duration, Number.isFinite(time) ? time : 0));
  PM.exprCache?.clear();
  PM.touch();
  PM.rasterClear?.();
  PM.bus.emit('layers'); PM.bus.emit('sel'); PM.bus.emit('assets'); PM.bus.emit('project');
  PM.invalidate();
  return next;
};
}
