import type { PMRegistry } from '../registry';
import { isProperty, resolveContent } from '../core/content-properties';
import { sourceTime } from '../core/retiming';
import { resolveMediaSource } from '../../media/media-path';
import type { AgentMediaSource, AgentMediaTiming } from '../../../../shared/media-tools';

/*
 * `__media_source`: the agent's media tools run in main against a file, so
 * the renderer names the file (resolveMediaSource) and, for a timeline clip,
 * how the clip's composition time maps onto that file's own time. Constant
 * speed and trim give two exact points; animated speed or time remapping are
 * sampled densely enough for word-level transcript mapping.
 */

const MAX_TIMING_SAMPLES = 2000;
const SAMPLES_PER_SECOND = 20;

function animated(value: unknown): boolean {
  return isProperty(value) && ((value as any).kf.length > 0 || !!(value as any).expr);
}

export function layerTiming(PM: PMRegistry, layer: any): AgentMediaTiming {
  const from = Number(layer.from) || 0;
  const duration = Math.max(0, Number(layer.dur) || 0);
  const content = resolveContent(PM, layer, from);
  const remapped = !!content.timeRemap || animated(layer.d?.timeRemap);
  const constant = !remapped && !animated(layer.d?.speed) && !animated(layer.d?.trim);
  const at = (time: number): [number, number] => [round(time), round(sourceTime(PM, layer, time))];
  if (constant) return { constant: true, samples: [at(from), at(from + duration)] };
  const count = Math.max(2, Math.min(MAX_TIMING_SAMPLES, Math.ceil(duration * SAMPLES_PER_SECOND) + 1));
  const samples: Array<[number, number]> = [];
  for (let index = 0; index < count; index++) samples.push(at(from + duration * index / (count - 1)));
  return { constant: false, samples };
}

function round(value: number): number {
  return Math.round(value * 10000) / 10000;
}

export async function agentMediaSource(PM: PMRegistry, args: Record<string, unknown>): Promise<AgentMediaSource> {
  const layerId = typeof args.layerId === 'string' && args.layerId ? args.layerId : undefined;
  let assetId = typeof args.assetId === 'string' && args.assetId ? args.assetId : undefined;
  let layer: any = null;
  if (layerId) {
    layer = PM.L(layerId);
    if (!layer) throw new Error(`Layer ${layerId} is not in this composition. Read get_project_state for layer IDs.`);
    if (layer.type !== 'video' && layer.type !== 'audio') throw new Error(`“${layer.name}” is a ${layer.type} layer. Media tools read video and audio clips; use render_frames to look at other layers.`);
    const layerAsset = typeof layer.d?.asset === 'string' ? layer.d.asset : null;
    if (!layerAsset) throw new Error(`“${layer.name}” has no media attached.`);
    if (assetId && assetId !== layerAsset) throw new Error(`Layer ${layerId} plays asset ${layerAsset}, not ${assetId}. Pass one or the other.`);
    assetId = layerAsset;
  }
  if (!assetId) throw new Error('Pass assetId (mediaAssets in get_project_state) or layerId of a video or audio clip.');
  const resolved = await resolveMediaSource(assetId);
  const meta = PM.proj.assets?.[assetId] || {};
  const runtime = PM.assets?.get?.(assetId);
  const duration = Number(runtime?.dur ?? meta.dur);
  const contentKey = typeof meta.fingerprint === 'string' && meta.fingerprint ? meta.fingerprint
    : typeof meta.storageKey === 'string' && meta.storageKey ? meta.storageKey : undefined;
  const source: AgentMediaSource = {
    path: resolved.path,
    origin: resolved.origin,
    ...(contentKey ? { contentKey } : {}),
    asset: {
      id: assetId, name: resolved.name, kind: resolved.kind,
      duration: Number.isFinite(duration) && duration > 0 ? duration : null,
      hasAudio: resolved.kind === 'audio' || runtime?.hasAudio === true || meta.hasAudio === true,
      proxy: resolved.proxy
    }
  };
  if (layer) {
    const timing = layerTiming(PM, layer);
    const sources = timing.samples.map(([, time]) => time);
    const limit = source.asset.duration ?? Infinity;
    source.layer = {
      id: layer.id, name: String(layer.name || layer.id), type: layer.type,
      from: Number(layer.from) || 0, duration: Number(layer.dur) || 0, timing,
      sourceStart: Math.max(0, Math.min(...sources)),
      sourceEnd: Math.min(limit, Math.max(...sources))
    };
  }
  return source;
}
