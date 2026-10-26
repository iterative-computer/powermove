import { layerInputPlan, hasLayerInputs } from '../../core/effect-inputs';
import { evaluatedValue } from './content-properties';
import { sequenceStreamTime } from '../../../../shared/image-sequence';
import { videoPlaybackTime } from '../../../../shared/video-timing';
import { sourceTime } from './retiming';
import { resolveContent } from './content-properties';

export type VideoClip = { layer: any; asset: any; id: string; time: number; at: number; streamAt: number | null; rate: number | null };

/** Resolve each visible decoder once, at the composition frame's central time.
 * A repeated precomp owns separate decoders, just as repeated media clips do. */
export function videoClipsAt(PM: any, time: number): VideoClip[] {
  const clips: VideoClip[] = [];
  const visit = (comp: any, at: number, prefix: string, rate: number | null, depth: number) => {
    if (depth > 8) return;
    PM.scope?.push(comp);
    try {
      const required = hasLayerInputs(comp, (fx:any)=>PM.effectDef?.(fx) ?? PM.Kernel?.effects?.get?.(fx.type)) ? layerInputPlan(comp, { assets:PM.assets, definition: (fx: any) => PM.effectDef?.(fx), enabled: (l: any, fx: any) => !!evaluatedValue(PM,l,fx.on,at,`${fx.id}.$enabled`), active: (l: any) => !!PM.active(l,at), contentActive: (l: any) => !!PM.active({...l,group:null},at) }).layerIds : new Set();
      const layers = PM.ProjectIndex?.layersOfType
        ? [...PM.ProjectIndex.layersOfType('video', comp), ...PM.ProjectIndex.layersOfType('precomp', comp)]
        : comp.layers;
      for (const layer of layers) {
        if (!PM.active(layer, at) && !(required.has(layer.id) && at >= layer.from-1e-6 && (at < layer.from+layer.dur-1e-6 || Math.abs(at-comp.dur)<=1e-6 && Math.abs(at-layer.from-layer.dur)<=1e-6))) continue;
        if (layer.type !== 'video' && layer.type !== 'precomp') continue;
        const d = resolveContent(PM, layer, at);
        const variable = d.timeRemap || layer.d.speed?.kf?.length || layer.d.speed?.expr;
        const speed = Number(d.speed ?? 1);
        const nextRate = rate == null || variable || speed <= 0 ? null : rate * speed;
        const source = sourceTime(PM, layer, at);
        const id = prefix + layer.id;
        if (layer.type === 'video') {
          const asset = PM.assets.get(d.asset);
          if (asset?.el) clips.push({ layer, asset, id, time: at,
            streamAt: sequenceStreamTime(asset, source) ?? null,
            at: videoPlaybackTime(asset, source), rate: nextRate });
        } else {
          const sub = comp.comps?.[d.comp] || PM.proj.comps?.[d.comp];
          if (sub) visit(sub, source, id + '/', nextRate, depth + 1);
        }
      }
    } finally { PM.scope?.pop(); }
  };
  visit(PM.proj, time, '', 1, 0);
  return clips;
}
