import {effectMightRender} from '../../core/effect-inputs';
/** Conservative temporal analysis for editable, built-in artwork. Unknown
 * renderers, expressions and media always retain the normal per-frame path. */
export function previewPlan(project: any, effectStationary?: (effect: any) => boolean) {
  const edges = new Set<number>();
  const moving: Array<[number, number]> = [];
  let continuous = false;
  const staticLayers = new Set<any>();
  const independent = new Set<any>();
  const layerMotion = new Map<any, Array<[number, number]>>();
  const expressions = new Set<any>();
  for (const layer of project.layers || []) {
    // Sound keeps advancing independently of the retained picture.
    if (layer.type === 'audio') continue;
    const activeEffects=(layer.fx??[]).filter(effectMightRender);
    let constant = true;
    const ranges: Array<[number, number]> = [];
    layerMotion.set(layer, ranges);
    const from = Number(layer.from) || 0, end = from + (Number(layer.dur) || 0);
    edges.add(from); edges.add(end);
    if (!['shape', 'text', 'solid', 'group', 'null'].includes(layer.type)
        || activeEffects.length || layer.transitionIn || layer.transitionOut
        || layer.d?.animators?.length || layer.threeD) {
      continuous = true;
    }
    const seen = new Set<object>();
    const visit = (value: any) => {
      if (!value || typeof value !== 'object' || seen.has(value)) return;
      seen.add(value);
      if (value.expr) { continuous = true; constant = false; expressions.add(layer); }
      if (Array.isArray(value.kf) && value.kf.length > 1) {
        constant = false;
        for (let i = 1; i < value.kf.length; i++) {
          const a = value.kf[i - 1], b = value.kf[i];
          const start = Number(a.t) + from, finish = Number(b.t) + from;
          if (!Number.isFinite(start) || !Number.isFinite(finish) || finish < start) { continuous = true; expressions.add(layer); continue; }
          edges.add(start); edges.add(finish);
          const held = a.hold || typeof a.v === 'number' && typeof b.v === 'number'
            && (a.outInterp === 'hold' || b.inInterp === 'hold');
          // Equal endpoints can still move with native speed handles. Reuse
          // only proven flat segments, never inferred easing overshoots.
          const flat = Object.is(a.v, b.v) && !a.autoBezier && !b.autoBezier
            && !Number(a.outEase?.speed) && !Number(b.inEase?.speed);
          if (!held && !flat) { moving.push([start, finish]); ranges.push([start, finish]); }
        }
      }
      for (const [key,child] of Object.entries(value)) {
        if(key==='kf')continue;
        if(value===layer&&key==='fx'){for(const effect of activeEffects)visit(effect);}else visit(child);
      }
    };
    if (['shape', 'text', 'solid', 'image', 'group', 'null'].includes(layer.type)) visit(layer);
    else {
      // Unknown renderers only participate through proven invisibility. Their
      // arbitrary source data does not need to be inspected to retain pixels
      // beneath a disabled or zero-opacity layer.
      visit(layer.on); visit(layer.p?.opacity);
    }
    if (constant && ['shape', 'text', 'solid', 'group', 'null'].includes(layer.type)
        && !activeEffects.length && !layer.transitionIn && !layer.transitionOut && !layer.d?.animators?.length
        && !layer.threeD && !layer.matteSource && !layer.masks?.length && !layer.solo) independent.add(layer);
  }
  const byId = new Map<any, any>((project.layers || []).map((layer: any) => [layer.id, layer]));
  const resolved = new Map<any, boolean>(), visiting = new Set<any>();
  const stable = (layer: any): boolean => {
    if (resolved.has(layer)) return resolved.get(layer)!;
    if (!independent.has(layer) || visiting.has(layer)) return false;
    visiting.add(layer);
    const value = (!layer.parent || stable(byId.get(layer.parent))) && (!layer.group || stable(byId.get(layer.group)));
    visiting.delete(layer); resolved.set(layer, value);
    return value;
  };
  for (const layer of independent) if (layer.type !== 'group' && stable(layer)) staticLayers.add(layer);
  const boundaries = [...edges].filter(Number.isFinite).sort((a, b) => a - b);
  moving.sort((a, b) => a[0] - b[0]);
  const intervals: Array<[number, number]> = [];
  for (const range of moving) {
    const last = intervals.at(-1);
    if (last && range[0] <= last[1]) last[1] = Math.max(last[1], range[1]);
    else intervals.push([...range]);
  }
  const upper = (values: number[], time: number) => {
    let lo = 0, hi = values.length;
    while (lo < hi) { const mid = (lo + hi) >>> 1; if (values[mid]! <= time) lo = mid + 1; else hi = mid; }
    return lo;
  };
  const starts = intervals.map(range => range[0]);
  // Settled entrances and held segments are as reusable as unkeyed artwork.
  // Keep each layer's ranges separate: a moving foreground must not prevent
  // retaining the completed picture beneath it.
  const tracks = new Map<any, { ranges: Array<[number, number]>; starts: number[] }>();
  for (const [layer, ranges] of layerMotion) {
    ranges.sort((a, b) => a[0] - b[0]);
    const merged: Array<[number, number]> = [];
    for (const range of ranges) {
      const last = merged.at(-1);
      if (last && range[0] <= last[1]) last[1] = Math.max(last[1], range[1]);
      else merged.push([...range]);
    }
    tracks.set(layer, { ranges: merged, starts: merged.map(range => range[0]) });
  }
  const heldValue = (prop: any, local: number): any => {
    if (!prop || typeof prop !== 'object') return prop;
    if (prop.expr) return undefined;
    const keys = prop.kf;
    if (!Array.isArray(keys) || !keys.length) return prop.v;
    if (local <= keys[0].t) return keys[0].v;
    if (local >= keys.at(-1).t) return keys.at(-1).v;
    let lo = 0, hi = keys.length - 1;
    while (hi - lo > 1) { const mid = (lo + hi) >>> 1; keys[mid].t <= local ? lo = mid : hi = mid; }
    const a = keys[lo], b = keys[hi];
    if (a.hold || a.outInterp === 'hold' || b.inInterp === 'hold'
        || typeof a.v === 'boolean' || typeof a.v === 'string') return a.v;
    return Object.is(a.v, b.v) && !a.autoBezier && !b.autoBezier
      && !a.spring && !Number(a.outEase?.speed) && !Number(b.inEase?.speed) ? a.v : undefined;
  };
  return {
    staticLayers,
    // Prefix caching uses visibility intervals even when the foreground moves.
    interval(time: number, radius = 0): number | null {
      const index = upper(boundaries, time);
      if (Math.abs(time - (boundaries[index - 1] ?? -Infinity)) <= radius + 2e-6
          || Math.abs(time - (boundaries[index] ?? Infinity)) <= radius + 2e-6) return null;
      return index;
    },
    timeKey(time: number, radius = 0): string | number {
      if (continuous) return time;
      const range = intervals[upper(starts, time + radius) - 1];
      if (range && time - radius <= range[1]) return time;
      const index = this.interval(time, radius);
      return index === null ? time : `still:${index}`;
    },
    prefixStart(layers: any[], time: number, radius = 0): number {
      if (this.interval(time, radius) === null) return layers.length;
      const memo = new Map<any, boolean>(), path = new Set<any>();
      const settled = (layer: any): boolean => {
        if (!layer) return false;
        if (memo.has(layer)) return memo.get(layer)!;
        if (path.has(layer)) return false;
        path.add(layer);
        const track = tracks.get(layer), range = track?.ranges[upper(track.starts, time + radius) - 1];
        const value = !!track && !expressions.has(layer)
          && ['shape', 'text', 'solid', 'image', 'group', 'null'].includes(layer.type)
          && !layer.transitionIn && !layer.transitionOut && !layer.d?.animators?.length
          && !layer.threeD && !layer.matteSource && !layer.masks?.length && !layer.solo
          && (!range || time - radius > range[1])
          && (layer.fx || []).every((effect: any) => !effectMightRender(effect) || effectStationary?.(effect))
          && (!layer.parent || settled(byId.get(layer.parent)))
          && (!layer.group || settled(byId.get(layer.group)));
        path.delete(layer); memo.set(layer, value); return value;
      };
      const hidden = (layer: any): boolean => {
        if (layer.type === 'audio' || layer.type === 'null') return true;
        if (layer.type !== 'group' && (time + radius < Number(layer.from) || time - radius >= Number(layer.from) + Number(layer.dur))) return true;
        const local = time - (Number(layer.from) || 0);
        if (layer.on === false || Array.isArray(layer.on?.kf) && heldValue(layer.on, local) === false) return true;
        const opacity = layer.p?.opacity && typeof layer.p.opacity === 'object' ? heldValue(layer.p.opacity, local) : undefined;
        return typeof opacity === 'number' && opacity <= .1;
      };
      let start = layers.length;
      while (start > 0) {
        const layer = layers[start - 1];
        // Group boundaries can contain moving descendants, even with a
        // stationary group transform. Only proven invisible groups are skipped.
        if (!hidden(layer) && (layer.type === 'group' || !settled(layer))) break;
        start--;
      }
      return start;
    },
  };
}
