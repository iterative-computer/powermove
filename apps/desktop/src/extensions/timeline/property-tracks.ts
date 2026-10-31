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

/** Position's axes are one property unless the layer separates its
 * dimensions (After Effects' Separate Dimensions). Projects without the flag
 * keep separate rows, as they always had. */
export const positionPaths = ['position.x', 'position.y', 'position.z'];

export function dimensionsSeparated(layer: any): boolean {
  return layer?.separateDimensions !== false;
}

/** A row key that has dimensions to separate or join. */
export function isPositionPath(key: unknown): boolean {
  return key === 'position' || positionPaths.includes(String(key));
}

/** Merge axis rows into one presentation row whose keys are grouped by time. */
function joinAxes(props: any[], paths: string[], key: string, label: string): any[] {
  const axes = paths.map(path => props.find((p: any) => p.key === path)).filter(Boolean);
  if (axes.length < 2) return props;
  const times = new Map<number, any[]>();
  for (const axis of axes) for (const k of axis.prop.kf) {
    // Keys are frame-snapped on creation. Keep genuinely distinct legacy times.
    const members = times.get(k.t) ?? [];
    members.push({ key: k, prop: axis.prop, path: axis.key });
    times.set(k.t, members);
  }
  const kf = [...times.values()].map(members => ({
    ...members[0].key,
    hold: members.every(member => member.key.hold),
    members
  })).sort((a, b) => a.t - b.t);
  // The joined row takes the place of whichever axis is listed first.
  const first = props.find((p: any) => axes.includes(p));
  const rest = new Set(axes.filter((axis: any) => axis !== first).map((axis: any) => axis.key));
  return props.filter((p: any) => !rest.has(p.key)).map((p: any) => p === first ? {
    ...p, key, label, channels: axes,
    prop: { kf, expr: axes.find((axis: any) => axis.prop.expr)?.prop.expr ?? null }
  } : p);
}

export function timelineProperties(api: Pick<PowermoveAPI, 'anim' | 'space3d'>, layer: any, space3d: Pick<Space3DAPI, 'CHANNELS_3D'> = api.space3d): any[] {
  const channels3d = space3d.CHANNELS_3D ?? COMPATIBILITY_3D_CHANNELS;
  let props = api.anim.allProps(layer).filter((p: any) => layer.threeD || !(p.key in channels3d));
  if (props.some((p: any) => p.key === scalePaths[0]) && props.some((p: any) => p.key === scalePaths[1])) {
    props = joinAxes(props, scalePaths, 'scale', 'Scale');
  }
  if (!dimensionsSeparated(layer)) props = joinAxes(props, positionPaths, 'position', 'Position');
  return props;
}

export function trackSelected(row: any, channel: string): boolean {
  return row.key === channel || trackChannels(row).some(p => p.key === channel);
}

/** Selecting a Scale key always selects both stored dimensions at that time,
 * and so does a key of a Position whose dimensions are joined. */
export function expandScaleKeyIds(api: Pick<PowermoveAPI, 'project'>, keys: any[]): string[] {
  const ids = new Set<string>(keys.flatMap(key => typeof key === 'string'
    ? [key] : keyMembers(key).map(member => member.key.i)).filter(Boolean));
  for (const layer of api.project.get().layers) {
    const properties = layer.p as Record<string, { kf: any[] }>;
    const groups = dimensionsSeparated(layer) ? [scalePaths] : [scalePaths, positionPaths];
    for (const paths of groups) {
      const axes = paths.map(path => properties[path]).filter((property): property is { kf: any[] } => property != null);
      const times = new Set<number>();
      for (const prop of axes) for (const key of prop.kf) if (ids.has(key.i)) times.add(key.t);
      for (const prop of axes) for (const key of prop.kf) if (times.has(key.t)) ids.add(key.i);
    }
  }
  return [...ids];
}
