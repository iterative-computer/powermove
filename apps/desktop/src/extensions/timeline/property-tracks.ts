import { CHANNELS_3D } from 'powermove';
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

export function timelineProperties(PM: any, layer: any): any[] {
  const props = PM.allProps(layer).filter((p: any) => layer.threeD || !(p.key in CHANNELS_3D));
  const axes = scalePaths.map(path => props.find((p: any) => p.key === path)).filter(Boolean);
  if (axes.length !== 2) return props;
  const times = new Map<number, any[]>();
  for (const axis of axes) for (const key of axis.prop.kf) {
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
    ...p, key: 'scale', label: 'Scale', channels: axes,
    prop: { kf, expr: axes.find(axis => axis.prop.expr)?.prop.expr ?? null }
  } : p);
}

export function trackSelected(row: any, channel: string): boolean {
  return row.key === channel || trackChannels(row).some(p => p.key === channel);
}

/** Selecting a Scale key always selects both stored dimensions at that time. */
export function expandScaleKeyIds(PM: any, keys: any[]): string[] {
  const ids = new Set<string>(keys.flatMap(key => typeof key === 'string'
    ? [key] : keyMembers(key).map(member => member.key.i)).filter(Boolean));
  for (const layer of PM.proj.layers) {
    const axes = scalePaths.map(path => layer.p?.[path]).filter(Boolean);
    const times = new Set<number>();
    for (const prop of axes) for (const key of prop.kf) if (ids.has(key.i)) times.add(key.t);
    for (const prop of axes) for (const key of prop.kf) if (times.has(key.t)) ids.add(key.i);
  }
  return [...ids];
}
