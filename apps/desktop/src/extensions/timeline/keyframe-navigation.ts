import { timelineProperties, trackChannels, trackSelected } from './property-tracks';

/** Key times stay layer-local in the document; navigation uses composition time. */
export function keyframeTimes(PM: any, row?: any): number[] {
  const selected = new Set(PM.sel?.layers ?? []);
  const layers = (PM.proj?.layers ?? []).filter((L: any) => !L.shy && (!selected.size || selected.has(L.id)));
  const rows = row ? [row] : layers.flatMap((L: any) => timelineProperties(PM, L)
    .filter((p: any) => !PM.sel?.chan || trackSelected(p, PM.sel.chan))
    .map((p: any) => ({ ...p, L })));
  const times: number[] = rows.flatMap((r: any) => trackChannels(r).flatMap(axis =>
    (axis.prop.kf ?? []).map((key: any) => Number(r.L.from) + Number(key.t))));
  return [...new Set(times.filter(time => Number.isFinite(time) && time >= 0 && time <= PM.proj.dur))].sort((a, b) => a - b);
}

export function adjacentKeyframe(PM: any, direction: -1 | 1, row?: any): number | undefined {
  const times = keyframeTimes(PM, row);
  return direction === 1 ? times.find(time => time > PM.time + 1e-5)
    : times.reverse().find(time => time < PM.time - 1e-5);
}
