/*
 * Which layers get a channel strip, and what each strip shows. One strip per
 * audible top-level layer of the open composition, in timeline order: audio
 * layers with media, videos that carry a soundtrack, and precomps whose
 * composition (at any depth) has sound. Pure over the project so it can be
 * tested without a renderer.
 */
import { isProperty } from '../legacy/core/content-properties';
import { clampGain } from './levels';

export type StripKind = 'audio' | 'video' | 'precomp';

/** Why a strip is silent although its own mute is off. */
export type StripSilence = 'solo' | 'off' | null;

export interface MixerStrip {
  id: string;
  name: string;
  kind: StripKind;
  /** The content field that holds this strip's level. */
  gainKey: 'gain' | 'audioGain';
  /** Linear level at the playhead. */
  gain: number;
  /** The level is keyframed; edits write a keyframe at the playhead. */
  animated: boolean;
  /** A keyframe sits exactly at the playhead. */
  keyAtTime: boolean;
  muted: boolean;
  /** Mute is animated on the timeline, so the strip cannot toggle it. */
  muteAnimated: boolean;
  locked: boolean;
  silencedBy: StripSilence;
}

export interface StripContext {
  time: number;
  comps: Record<string, any> | undefined;
  /** Evaluate a static value or an animated channel of `layer` at `time`. */
  evaluate(layer: any, value: unknown, time: number, path: string): unknown;
  hasKeyAt(layer: any, prop: any, time: number): unknown;
  groupAncestors(layer: any, layers: any[]): any[];
}

const MAX_DEPTH = 8;

/** True when `layer` contributes sound to the mix (ignoring mutes). */
export function layerCarriesAudio(layer: any, comps: Record<string, any> | undefined, depth = 0, seen: Set<string> = new Set()): boolean {
  if (!layer || !layer.d) return false;
  if (layer.type === 'audio') return typeof layer.d.asset === 'string' && !!layer.d.asset;
  if (layer.type === 'video') return layer.d.embeddedAudio === true && !!layer.d.asset;
  if (layer.type !== 'precomp' || typeof layer.d.comp !== 'string') return false;
  return compCarriesAudio(comps?.[layer.d.comp], comps, depth + 1, seen);
}

export function compCarriesAudio(comp: any, comps: Record<string, any> | undefined, depth = 0, seen: Set<string> = new Set()): boolean {
  if (!comp || !Array.isArray(comp.layers) || depth > MAX_DEPTH) return false;
  const id = typeof comp.id === 'string' ? comp.id : '';
  if (id && seen.has(id)) return false;
  const next = id ? new Set([...seen, id]) : seen;
  return comp.layers.some((layer: any) => layerCarriesAudio(layer, comps, depth, next));
}

function stripKind(layer: any): StripKind {
  return layer.type === 'audio' ? 'audio' : layer.type === 'video' ? 'video' : 'precomp';
}

export function deriveStrips(project: any, context: StripContext): MixerStrip[] {
  const layers: any[] = Array.isArray(project?.layers) ? project.layers : [];
  const time = context.time;
  const soloing = layers.some((layer) => layer?.solo);
  const strips: MixerStrip[] = [];
  for (const layer of layers) {
    if (!layerCarriesAudio(layer, context.comps)) continue;
    const kind = stripKind(layer);
    const gainKey = kind === 'audio' ? 'gain' : 'audioGain';
    const raw = layer.d[gainKey];
    const animated = isProperty(raw) && raw.kf.length > 0;
    const evaluated = Number(context.evaluate(layer, raw, time, `c.${gainKey}`));
    const gain = clampGain(Number.isFinite(evaluated) ? evaluated : 1);
    const groups = context.groupAncestors(layer, layers) || [];

    let muted: boolean;
    let muteAnimated = false;
    if (kind === 'audio') {
      muteAnimated = isProperty(layer.on);
      muted = context.evaluate(layer, layer.on, time, 'l.on') === false;
    } else {
      muted = layer.d.audioMuted === true;
    }

    let silencedBy: StripSilence = null;
    const switchedOff = (kind !== 'audio' && context.evaluate(layer, layer.on, time, 'l.on') === false)
      || groups.some((group: any) => context.evaluate(group, group.on, time, 'l.on') === false);
    if (switchedOff) silencedBy = 'off';
    else if (soloing && !layer.solo && !groups.some((group: any) => group.solo)) silencedBy = 'solo';

    strips.push({
      id: String(layer.id),
      name: String(layer.name || (kind === 'audio' ? 'Audio' : kind === 'video' ? 'Video' : 'Composition')),
      kind,
      gainKey,
      gain,
      animated,
      keyAtTime: animated && !!context.hasKeyAt(layer, raw, time),
      muted,
      muteAnimated,
      locked: !!layer.lock || groups.some((group: any) => group.lock),
      silencedBy
    });
  }
  return strips;
}

/** A cheap identity for the strip list, so the panel rebuilds rows only when it changes. */
export function stripsKey(strips: readonly MixerStrip[]): string {
  return strips.map((strip) => strip.id).join('\u0000');
}
