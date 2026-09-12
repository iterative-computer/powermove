import type { PMRegistry } from '../registry';
import { isProperty, resolveContent } from '../core/content-properties';

function number(value: unknown, name: string, min = 0): number {
  if (typeof value !== 'number' || !Number.isFinite(value) || value < min) throw new Error(`${name} must be a finite number >= ${min}.`);
  return value;
}
function staticTiming(layer: any): void {
  for (const key of ['trim', 'speed', 'sourceTime', 'timeRemap']) {
    const value = layer.d[key];
    if ((isProperty(value) && (value.kf.length || value.expr)) || (key === 'timeRemap' && (isProperty(value) ? value.v : value))) {
      throw new Error('This clip has animated timing or time remapping. Use property/keyframe commands to edit its timing explicitly.');
    }
  }
}
function rebase(PM: PMRegistry, layer: any, offset: number): void {
  for (const { prop } of PM.allProps(layer)) for (const key of prop.kf || []) key.t -= offset;
}

/** Trusted structural edits retain the entire layer, including effects and masks.
 * The caller supplies the agent run's revision guard and history group. */
export function editVideo(PM: PMRegistry, args: Record<string, any>, meta: Record<string, any>): any {
  const operation = args.operation;
  const supported = ['insert', 'split', 'trim', 'move', 'speed', 'remove', 'separate_audio', 'audio'];
  if (!supported.includes(operation)) throw new Error('Unknown video editing operation.');
  const layers = PM.proj.layers;
  const layer = layers.find((item: any) => item.id === args.layerId);
  if (operation !== 'insert' && (!layer || !['video', 'audio'].includes(layer.type))) throw new Error('Choose a video or audio layer from get_project_state.');
  if (layer?.lock) throw new Error('The clip is locked.');
  if (layer && Object.values(layer.locked_intent || {}).some(Boolean)) throw new Error('The clip contains protected hand edits. Edit unprotected channels explicitly with apply_commands.');
  return PM.Edit.mutate(meta.label, () => {
    if (operation === 'insert') {
      const asset = PM.proj.assets?.[args.assetId];
      const runtime = PM.assets?.get?.(args.assetId);
      if (!asset || !['video', 'audio'].includes(asset.kind)) throw new Error('Choose an imported video/audio asset from get_project_state.');
      const from = number(args.from, 'from');
      const trim = number(args.sourceIn ?? 0, 'sourceIn');
      const duration = number(args.duration, 'duration', 1 / PM.proj.fps);
      const sourceDuration = runtime?.dur ?? asset.dur;
      if (!Number.isFinite(sourceDuration) || trim + duration > sourceDuration + 1e-6) throw new Error('The requested clip exceeds the available source duration.');
      if (from + duration > PM.proj.dur + 1e-6) throw new Error('Extend the composition before inserting beyond its end.');
      const clip = PM.mkLayer(asset.kind, { name: asset.name, from, dur: duration, d: {
        asset: args.assetId, trim: PM.P(trim), ...(asset.kind === 'video' ? { speed: PM.P(1), embeddedAudio: runtime?.hasAudio === true } : {})
      } });
      layers.unshift(clip);
      return { layerId: clip.id };
    }
    const end = layer.from + layer.dur;
    const d = resolveContent(PM, layer, layer.from);
    const rate = layer.type === 'video' ? Number(d.speed ?? 1) : 1;
    if (['split', 'trim', 'speed', 'separate_audio'].includes(operation)) {
      staticTiming(layer);
      if (!(rate > 0)) throw new Error('This clip needs explicit time-remapping edits.');
    }
    if (operation === 'move') {
      const from = number(args.from, 'from');
      if (from + layer.dur > PM.proj.dur + 1e-6) throw new Error('Extend the composition before moving beyond its end.');
      layer.from = from;
    } else if (operation === 'trim') {
      const start = number(args.start, 'start'), stop = number(args.end, 'end');
      if (start < layer.from || stop > end || stop - start < 1 / PM.proj.fps - 1e-6) throw new Error('Trim must retain at least one frame inside the current clip.');
      const offset = start - layer.from;
      rebase(PM, layer, offset);
      layer.d.trim = PM.P(Number(d.trim || 0) + offset * rate);
      layer.from = start; layer.dur = stop - start;
    } else if (operation === 'split') {
      const at = number(args.at, 'at');
      if (at - layer.from < 1 / PM.proj.fps - 1e-6 || end - at < 1 / PM.proj.fps - 1e-6) throw new Error('Split must leave at least one frame on each side.');
      const tail = PM.cloneLayer(layer);
      const offset = at - layer.from;
      rebase(PM, tail, offset);
      tail.from = at; tail.dur = end - at;
      tail.d.trim = PM.P(Number(d.trim || 0) + offset * rate);
      layer.dur = offset;
      layer.transitionOut = null; tail.transitionIn = null;
      if (layer.type === 'audio') { layer.d.fadeOut = PM.P(0); tail.d.fadeIn = PM.P(0); }
      layers.splice(layers.indexOf(layer), 0, tail);
      return { layerId: layer.id, tailId: tail.id };
    } else if (operation === 'speed') {
      if (layer.type !== 'video') throw new Error('Playback speed is supported for video clips.');
      if (d.embeddedAudio === true) throw new Error('Separate embedded audio first; retimed embedded soundtracks are not supported.');
      const speed = number(args.speed, 'speed', 0.01);
      const duration = layer.dur * rate / speed;
      if (duration < 1 / PM.proj.fps || layer.from + duration > PM.proj.dur + 1e-6) throw new Error('The retimed clip must fit the composition and retain at least one frame.');
      for (const { prop } of PM.allProps(layer)) for (const key of prop.kf || []) key.t *= rate / speed;
      layer.d.speed = PM.P(speed); layer.dur = duration;
    } else if (operation === 'separate_audio') {
      if (layer.type !== 'video' || d.embeddedAudio !== true) throw new Error('This video has no enabled embedded audio.');
      if (rate !== 1) throw new Error('Separate audio before retiming; audio layers do not support playback speed.');
      const audio = PM.mkLayer('audio', { name: `${layer.name} Audio`, from: layer.from, dur: layer.dur, d: { asset: d.asset, trim: PM.P(Number(d.trim || 0)) } });
      layer.d.embeddedAudio = false;
      layers.splice(layers.indexOf(layer), 0, audio);
      return { layerId: layer.id, audioId: audio.id };
    } else if (operation === 'audio') {
      if (layer.type !== 'audio') throw new Error('Separate embedded audio before adjusting gain or fades.');
      let count = 0;
      for (const key of ['gain', 'fadeIn', 'fadeOut']) if (args[key] !== undefined) {
        const value = number(args[key], key);
        if (key === 'gain' && value > 4) throw new Error('Audio gain must be between 0 and 4.');
        if (key !== 'gain' && value > layer.dur) throw new Error('Audio fades cannot exceed clip duration.');
        if (isProperty(layer.d[key]) && (layer.d[key].kf.length || layer.d[key].expr)) throw new Error('Use property/keyframe commands to edit animated audio controls.');
        layer.d[key] = PM.P(value); count++;
      }
      if (!count) throw new Error('Supply gain, fadeIn, or fadeOut.');
    } else if (operation === 'remove') {
      PM.removeLayers([layer.id]);
    }
    return { layerId: layer.id };
  }, meta);
}

export function videoAssets(PM: PMRegistry): any[] {
  return Object.values(PM.proj.assets || {}).filter((asset: any) => ['video', 'audio'].includes(asset.kind)).map((asset: any) => {
    const runtime = PM.assets?.get?.(asset.id);
    return { id: asset.id, name: asset.name, kind: asset.kind, duration: runtime?.dur ?? asset.dur ?? null, width: asset.w, height: asset.h, available: !!runtime, hasAudio: runtime?.hasAudio === true };
  });
}
