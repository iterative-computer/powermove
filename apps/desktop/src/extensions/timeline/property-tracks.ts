import type { PowermoveAPI, Space3DAPI } from 'powermove';

const COMPATIBILITY_3D_CHANNELS = {
  perspective: 50,
  'position.z': 0,
  'anchor.z': 0,
  'scale.z': 100,
  'rotation.x': 0,
  'rotation.y': 0,
  'orientation.x': 0,
  'orientation.y': 0,
  'orientation.z': 0,
} as const;
/** Presentation groups retain the real scalar channels and key IDs. No migration
 * or resampling is needed, including for old projects with unequal key times. */
export const scalePaths = ['scale.x', 'scale.y'];

export function trackChannels(row: any): any[] {
  return row.channels ?? [{ key: row.key, prop: row.prop, label: row.label }];
}

/** A handle belongs to its actual track, even when another graph is focused. */
export function graphHandleChannels(rows: any[], keyframes: any[]): any[] {
  const owner = rows.find(row => trackChannels(row).some(axis => axis.prop.kf === keyframes));
  return owner ? trackChannels(owner) : [];
}

export function keyMembers(key: any): any[] {
  return key.members ?? [{ key }];
}

export function timelineProperties(api: Pick<PowermoveAPI, 'anim' | 'space3d'>, layer: any, space3d: Pick<Space3DAPI, 'CHANNELS_3D'> = api.space3d): any[] {
  const channels3d = space3d.CHANNELS_3D ?? COMPATIBILITY_3D_CHANNELS;
  const props = api.anim.allProps(layer).filter((p: any) => layer.threeD || !(p.key in channels3d));
  const axes = scalePaths.map(path => props.find((p: any) => p.key === path)).filter(Boolean);
  const xAxis = axes[0], yAxis = axes[1];
  if (!xAxis || !yAxis) return props;
  const scaleAxes = [xAxis, yAxis];
  const times = new Map<number, any[]>();
  for (const axis of scaleAxes) for (const key of axis.prop.kf) {
    // Keys are frame-snapped on creation. Keep genuinely distinct legacy times.
    const members = times.get(key.t) ?? [];
    members.push({ key, prop: axis.prop, path: axis.key });
    times.set(key.t, members);
  }
  const kf = [...times.values()].map(members => ({
    ...members[0].key,
    hold: members.every(member => member.key.hold),
    members
  })).sort((a, b) => a.t - b.t);
  return props.filter((p: any) => p.key !== 'scale.y').map((p: any) => p.key === 'scale.x' ? {
    ...p, key: 'scale', label: 'Scale', channels: scaleAxes,
    prop: { kf, expr: scaleAxes.find(axis => axis.prop.expr)?.prop.expr ?? null }
  } : p);
}

export function trackSelected(row: any, channel: string): boolean {
  return row.key === channel || trackChannels(row).some(p => p.key === channel);
}

/** Selecting a Scale key always selects both stored dimensions at that time. */
export function expandScaleKeyIds(api: Pick<PowermoveAPI, 'project'>, keys: any[]): string[] {
  const ids = new Set<string>(keys.flatMap(key => typeof key === 'string'
    ? [key] : keyMembers(key).map(member => member.key.i)).filter(Boolean));
  for (const layer of api.project.get().layers) {
    const properties = layer.p as Record<string, { kf: any[] }>;
    const axes = scalePaths.map(path => properties[path]).filter((property): property is { kf: any[] } => property != null);
    const times = new Set<number>();
    for (const prop of axes) for (const key of prop.kf) if (ids.has(key.i)) times.add(key.t);
    for (const prop of axes) for (const key of prop.kf) if (times.has(key.t)) ids.add(key.i);
  }
  return [...ids];
}
