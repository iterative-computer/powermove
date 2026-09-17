import { timelineProperties, trackChannels, trackSelected } from './property-tracks';
import type { PowermoveAPI, Space3DAPI } from 'powermove';

/** Key times stay layer-local in the document; navigation uses composition time. */
export function keyframeTimes(api: Pick<PowermoveAPI, 'anim' | 'project' | 'selection' | 'space3d'>, row?: any, space3d: Pick<Space3DAPI, 'CHANNELS_3D'> = api.space3d): number[] {
  const selected = new Set(api.selection.layers());
  const layers = api.project.get().layers.filter((L) => !selected.size || selected.has(L.id));
  const rows = row ? [row] : layers.flatMap((L: any) => timelineProperties(api, L, space3d)
    .filter((p: any) => !api.selection.chan() || trackSelected(p, api.selection.chan()!))
    .map((p: any) => ({ ...p, L })));
  const times: number[] = rows.flatMap((r: any) => trackChannels(r).flatMap(axis =>
    (axis.prop.kf ?? []).map((key: any) => Number(r.L.from) + Number(key.t))));
  return [...new Set(times.filter(time => Number.isFinite(time) && time >= 0 && time <= api.project.get().dur))].sort((a, b) => a - b);
}

export function adjacentKeyframe(api: Pick<PowermoveAPI, 'anim' | 'project' | 'selection' | 'space3d' | 'transport'>, direction: -1 | 1, row?: any, space3d: Pick<Space3DAPI, 'CHANNELS_3D'> = api.space3d): number | undefined {
  const times = keyframeTimes(api, row, space3d);
  return direction === 1 ? times.find(time => time > api.transport.time() + 1e-5)
    : times.reverse().find(time => time < api.transport.time() - 1e-5);
}
