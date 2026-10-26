import type { CommandKeyframe, EditCommand, ReplaceKeyframesCommand } from '../../core/types/commands';
import type { Channel, Keyframe } from '../../core/types/project';
import { adoptTemporalEase, easeFromUnitHandle, linearKeyframeEase, segmentUnitCurve, setInterpolation,
  type EaseKeyframe, type TemporalEaseFields, type UnitCurve } from '../../core/anim/temporal-ease';
import { clone, cubic, normalize, point, type Curve } from './curve';

type PM = Record<string, any>;
export interface Track { layerId: string; from: number; path: string; keys: Keyframe[]; selected: string[] }
export interface Sections { incoming: boolean; middle: boolean; outgoing: boolean }
export const ALL_SECTIONS: Sections = { incoming: true, middle: true, outgoing: true };
export function sections(event?: Pick<MouseEvent, 'shiftKey' | 'altKey' | 'metaKey' | 'ctrlKey'>): Sections {
  if (!event || !(event.shiftKey || event.altKey || event.metaKey || event.ctrlKey)) return { ...ALL_SECTIONS };
  const mac = typeof navigator === 'undefined' || /Mac/i.test(navigator.platform);
  return { incoming: event.shiftKey, middle: mac ? event.altKey : event.ctrlKey, outgoing: mac ? event.metaKey : event.altKey };
}
export function tracks(PM: PM): Track[] {
  const ids = new Set<string>(PM.sel.keys), result: Track[] = [];
  for (const layer of PM.curComp().layers) {
    if (layer.lock || layer.locked) continue;
    for (const item of PM.allProps(layer)) {
      const selected = item.prop.kf.filter((key: Keyframe) => ids.has(key.i)).map((key: Keyframe) => key.i);
      if (selected.length) {
        const keys = clone(item.prop.kf) as Keyframe[];
        if (typeof item.prop.v === 'number') adoptTemporalEase(keys as unknown as Record<string, unknown>[]);
        result.push({ layerId: layer.id, from: layer.from, path: item.key, keys, selected });
      }
    }
  }
  return result;
}
export function temporal(key: Keyframe): TemporalEaseFields {
  return { inInterp: key.inInterp, outInterp: key.outInterp, inEase: clone(key.inEase), outEase: clone(key.outEase),
    autoBezier: key.autoBezier, continuous: key.continuous };
}
export function command(track: Track, keys = track.keys): ReplaceKeyframesCommand {
  return { type: 'replace_keyframes', target: track.layerId, path: track.path, preserveHandEdits: false,
    keyframes: keys.map(k => ({ time: k.t, value: k.v, id: k.i, spring: (k as any).spring || null,
      ...(typeof k.v === 'number' ? { temporal: temporal(k) } : {
        hold: (k as any).hold === true || k.outInterp === 'hold',
        handles: [...((k as any).eo || [0, 0]), ...((k as any).ei || [1, 1])] as UnitCurve,
      }) })) };
}
function selectedIndices(track: Track): number[] {
  return track.keys.flatMap((k, i) => track.selected.includes(k.i) ? [i] : []);
}
export function loadCurve(track: Track): Curve {
  let indices = selectedIndices(track);
  if (indices.length === 1) {
    const i = indices[0]!;
    indices = i < track.keys.length - 1 ? [i, i + 1] : [i - 1, i];
  }
  if (indices.length < 2 || indices[0]! < 0 || indices.some((v, i) => i && v !== indices[i - 1]! + 1)) throw new Error('Select consecutive keyframes from one property');
  return normalize(indices.map(i => track.keys[i]!) as EaseKeyframe[]);
}
function eligible(track: Track, i: number, side: 'in' | 'out', target: Sections): boolean {
  const ids = new Set(track.selected), key = track.keys[i]!, other = track.keys[i + (side === 'out' ? 1 : -1)];
  if (!ids.has(key.i) || !other) return false;
  return ids.has(other.i) ? target.middle : side === 'in' ? target.incoming : target.outgoing;
}
export function applyCurve(input: Track[], curve: Curve, uid: () => string, target = ALL_SECTIONS): ReplaceKeyframesCommand[] {
  if (!input.length) throw new Error('Select keyframes on an unlocked layer');
  const result: ReplaceKeyframesCommand[] = [];
  for (const source of input) {
    const track = clone(source), indices = selectedIndices(track);
    if (track.keys.some(k => typeof k.v !== 'number')) throw new Error('Easing requires numeric properties');
    if (curve.points.length === 2) {
      const handles = segmentUnitCurve(curve.points[0]!, curve.points[1]!)!;
      for (const i of indices) for (const side of ['in', 'out'] as const) {
        if (!eligible(track, i, side, target)) continue;
        const key = track.keys[i]!, other = track.keys[i + (side === 'out' ? 1 : -1)]!;
        const dt = Math.abs(other.t - key.t), dv = side === 'out' ? Number(other.v) - Number(key.v) : Number(key.v) - Number(other.v);
        key[`${side}Interp`] = curve.points[side === 'out' ? 0 : 1]![`${side}Interp`];
        key[`${side}Ease`] = easeFromUnitHandle(side === 'out' ? [handles[0], handles[1]] : [handles[2], handles[3]], dt, dv, side);
        key.autoBezier = key.continuous = false;
        if (side === 'out') delete (key as any).spring;
      }
    } else {
      if (indices.length < 2 || indices.some((v, i) => i && v !== indices[i - 1]! + 1)) throw new Error('Multi-keyframe curves need two or more consecutive selected keys');
      const start = indices[0]!, end = indices.at(-1)!, a = track.keys[start]!, b = track.keys[end]!;
      const dt = b.t - a.t, dv = Number(b.v) - Number(a.v);
      const next = curve.points.map((p, i): Keyframe => ({ ...clone(p), t: a.t + p.t * dt, v: Number(a.v) + p.v * dv,
        i: i === 0 ? a.i : i === curve.points.length - 1 ? b.i : uid(),
        inEase: { ...p.inEase, speed: p.inEase.speed * dv / dt }, outEase: { ...p.outEase, speed: p.outEase.speed * dv / dt } }));
      // External interpolation is outside the pasted span and remains untouched.
      Object.assign(next[0]!, { inInterp: a.inInterp, inEase: clone(a.inEase) });
      Object.assign(next.at(-1)!, { outInterp: b.outInterp, outEase: clone(b.outEase) });
      track.keys.splice(start, end - start + 1, ...next);
    }
    result.push(command(track));
  }
  return result;
}

export interface CopiedTrack { keys: Keyframe[]; all: Keyframe[]; path: string }
export function copyKeys(input: Track[]): CopiedTrack[] {
  if (!input.length) throw new Error('Select keyframes to copy');
  return input.map(t => ({ keys: clone(t.keys.filter(k => t.selected.includes(k.i))), all: clone(t.keys), path: t.path }));
}
export function pasteKeys(input: Track[], copied: CopiedTrack[], values: boolean, target = ALL_SECTIONS): ReplaceKeyframesCommand[] {
  if (!input.length || !copied.length) throw new Error('Copy keyframes, then select the destination keys');
  return input.map((source, ti) => {
    const track = clone(source), clipboard = copied.find(t => t.path === source.path) || copied[ti % copied.length]!;
    selectedIndices(track).forEach((i, n) => {
      const key = track.keys[i]!, origin = clipboard.keys[n % clipboard.keys.length]!;
      if (values) {
        if (typeof key.v !== typeof origin.v) throw new Error('Source and destination value types must match');
        key.v = origin.v;
      } else {
        if (typeof key.v !== 'number' || typeof origin.v !== 'number') throw new Error('Choose numeric properties to paste easing');
        const oi = clipboard.all.findIndex(k => k.i === origin.i);
        for (const side of ['in', 'out'] as const) {
          if (!eligible(track, i, side, target)) continue;
          const other = clipboard.all[oi + (side === 'out' ? 1 : -1)];
          const curve = other && (side === 'out' ? segmentUnitCurve(origin, other) : segmentUnitCurve(other, origin));
          const neighbour = track.keys[i + (side === 'out' ? 1 : -1)]!;
          const dt = Math.abs(neighbour.t - key.t), dv = side === 'out' ? Number(neighbour.v) - Number(key.v) : Number(key.v) - Number(neighbour.v);
          key[`${side}Interp`] = origin[`${side}Interp`];
          key[`${side}Ease`] = curve ? easeFromUnitHandle(side === 'out' ? [curve[0], curve[1]] : [curve[2], curve[3]], dt, dv, side) : clone(origin[`${side}Ease`]);
          key.autoBezier = key.continuous = false;
          if (side === 'out') delete (key as any).spring;
        }
      }
    });
    return command(track);
  });
}

export function duplicate(input: Track[], time: number, uid: () => string, reverse = false): ReplaceKeyframesCommand[] {
  if (!input.length) throw new Error('Select keyframes to duplicate');
  const all = input.flatMap(t => t.keys.filter(k => t.selected.includes(k.i)).map(k => k.t + t.from));
  const first = Math.min(...all), last = Math.max(...all);
  return input.map(source => {
    const track = clone(source);
    const next = track.keys.filter(k => track.selected.includes(k.i)).map(k => {
      const t = time - track.from + (reverse ? last - (k.t + track.from) : k.t + track.from - first);
      if (t < 0) throw new Error('Move the playhead inside the destination layers');
      return { ...clone(k), t, i: uid(), ...(reverse ? { inInterp: k.outInterp, outInterp: k.inInterp,
        inEase: { ...k.outEase, speed: -k.outEase.speed }, outEase: { ...k.inEase, speed: -k.inEase.speed } } : {}) };
    });
    track.keys = track.keys.filter(k => !next.some(n => Math.abs(n.t - k.t) < 1e-7)).concat(next).sort((a, b) => a.t - b.t);
    return command(track);
  });
}

export function interpolate(input: Track[], type: 'linear' | 'continuous' | 'auto' | 'hold'): ReplaceKeyframesCommand[] {
  if (!input.length) throw new Error('Select keyframes');
  return input.map(source => { const track = clone(source); for (const i of selectedIndices(track)) setInterpolation(track.keys, i, 'both', type); return command(track); });
}

/** Expressions stay editable; the raw keyframes remain the source of timing. */
export function expressionCommands(input: Track[], kind: 'overshoot' | 'bounce'): EditCommand[] {
  if (!input.length) throw new Error('Select numeric keyframes');
  return input.map(track => {
    if (track.keys.some(k => typeof k.v !== 'number')) throw new Error('Choose numeric properties');
    const selected = track.keys.filter(k => track.selected.includes(k.i));
    const last = selected.at(-1)!, before = selected.at(-2) || track.keys[track.keys.indexOf(last) - 1];
    if (!before || last.t <= before.t) throw new Error('Select a movement with at least two keys');
    const velocity = (Number(last.v) - Number(before.v)) / (last.t - before.t), elapsed = `(t-${last.t})`;
    const tail = kind === 'overshoot' ? `${velocity}*sin(${elapsed}*18)*exp(-${elapsed}*7)/18` : `${-velocity}*abs(sin(${elapsed}*15))*exp(-${elapsed}*6)/15`;
    return { type: 'set_expression', target: track.layerId, path: track.path, expression: `t<=${last.t}?value:value+${tail}` };
  });
}
