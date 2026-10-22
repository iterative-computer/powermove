import type { EffectRef } from './export-effects';
import { trackExportMedia } from './export-media';
import { staticEffectSupported, trackStaticEffects, type StaticMaskProgram } from './static-effects';

const record = (value: any) => value && typeof value === 'object' && !Array.isArray(value);
const builtinMask = (mask: any) => record(mask) && ['rect', 'ellipse'].includes(mask.shape)
  && mask.on === true && ['add', 'subtract'].includes(mask.mode) && record(mask.p)
  && (mask.path == null || record(mask.path) && record(mask.path.p) && Array.isArray(mask.path.vertices)
    && mask.path.vertices.every((vertex: any) => record(vertex) && record(vertex.p)));

/** Built-in artwork with no temporal inputs can share captured pixels. Images
 * require an explicit source check; other media and renderers stay per-frame. */
export function staticExportPlan(project: any, motionBlur: boolean, imageEligible?: (layer: any) => boolean,
  effectEligible?: (effect: any) => boolean, maskEligible?: (mask: any) => boolean) {
  const edges = new Set<number>();
  let blur = false;
  const constant = (value: any, seen = new Set<object>()): boolean => {
    if (!value || typeof value !== 'object' || seen.has(value)) return true;
    seen.add(value);
    if (value.expr || Array.isArray(value.kf) && value.kf.length > 1) return false;
    return Object.values(value).every(child => constant(child, seen));
  };
  if (!constant(project.backgroundFill)) return null;
  for (const layer of project.layers || []) {
    if(layer.effectSpace==='source')return null;
    if (layer.type === 'audio') continue;
    const supported = ['solid', 'shape', 'text', 'null', 'group'].includes(layer.type)
      || layer.type === 'image' && imageEligible?.(layer);
    if (!supported
        || layer.fx != null && (!Array.isArray(layer.fx) || !layer.fx.every((effect: any) => effectEligible?.(effect)))
        || layer.transitionIn || layer.transitionOut
        || layer.threeD || layer.matteSource
        || layer.masks != null && (!Array.isArray(layer.masks) || !layer.masks.every((mask: any) => maskEligible?.(mask)))
        || layer.d?.animators?.length || !constant(layer)) return null;
    const from = Number(layer.from), end = from + Number(layer.dur);
    if (!Number.isFinite(from) || !Number.isFinite(end)) return null;
    // Match PM.active's tolerance at hard entrances and exits. A timestamp
    // just below a nominal edge can already belong to the next picture.
    edges.add(from - 1e-6); edges.add(end - 1e-6);
    blur ||= !!layer.mblur;
  }
  const boundaries = [...edges].sort((a, b) => a - b);
  const radius = motionBlur && blur ? Math.abs(project.shutter || .5) / Math.max(1, project.fps) : 0;
  return (time: number): string => {
    // PM.active holds a layer at the composition's exact final frame.
    if (Math.abs(time - project.dur) <= 1e-6) return `frame:${time}`;
    let interval = 0;
    for (const edge of boundaries) {
      // Blur can sample both sides of a layer entrance/exit. Never retain
      // those frames; ordinary hard boundaries belong to the new interval.
      if (radius && Math.abs(time - edge) <= radius + 2e-6) return `frame:${time}`;
      if (edge <= time) interval++; else break;
    }
    return `still:${interval}`;
  };
}

/** Scope retained pictures to one export. Font and renderer changes can alter
 * otherwise constant artwork without changing its animation properties. */
export function trackStaticExport(PM: any, motionBlur: boolean) {
  let project: any, version: unknown, generation = 0, dirty = true, disposed = false;
  let layers: any, layerCount: number | undefined, background: any;
  let plan: ReturnType<typeof staticExportPlan> = null;
  let imageLayers: any[] = [], imageAvailabilityChecked = false;
  let media: ReturnType<typeof trackExportMedia> | undefined;
  let imageSources: ReturnType<ReturnType<typeof trackExportMedia>['sources']> = null;
  let effectIds: EffectRef[] = [], effects: ReturnType<typeof trackStaticEffects> | undefined;
  let maskPrograms: StaticMaskProgram[] = [];
  let effectStamp: ReturnType<ReturnType<typeof trackStaticEffects>['capture']> = null;
  let fonts: FontFaceSet | undefined, fontCount: number | undefined;
  const invalidate = () => { generation++; dirty = true; };
  const fontEvents = ['loading', 'loadingdone', 'loadingerror'];
  const subscriptions = [
    ...['fonts', 'layers', 'project', 'draw'].map(event => PM.bus?.on?.(event, invalidate)),
    ...['extensions:changed', 'theme:changed'].map(event => PM.Kernel?.events?.on?.(event, invalidate)),
  ];
  return {
    get revision() { return generation; },
    get hasStaticPlan() { return plan !== null; },
    // Images, effects and masks observe source edits after each write ACK.
    // Ordinary artwork retains its existing bounded repeat batching.
    get maxRepeat() { return plan && (imageLayers.length || effectIds.length || maskPrograms.length) ? 1 : 8; },
    key(time: number): string | null {
      if (disposed) return null;
      const currentFonts = typeof document === 'undefined' ? undefined : document.fonts;
      if (fonts !== currentFonts) {
        for (const event of fontEvents) fonts?.removeEventListener?.(event, invalidate);
        fonts = currentFonts;
        for (const event of fontEvents) fonts?.addEventListener?.(event, invalidate);
        invalidate();
      }
      if (fontCount !== fonts?.size) { fontCount = fonts?.size; invalidate(); }
      if (project !== PM.proj || version !== PM.animVersion?.() || dirty
          || layers !== PM.proj.layers || layerCount !== PM.proj.layers?.length || background !== PM.proj.backgroundFill) {
        project = PM.proj; version = PM.animVersion?.(); dirty = false;
        layers = project.layers; layerCount = layers?.length; background = project.backgroundFill;
        imageLayers = [];
        const ids = new Set<EffectRef>(), maskKeys = new Set<StaticMaskProgram>();
        for (const layer of project.layers || []) {
          if (layer.type === 'image') imageLayers.push(layer);
          if (layer.type !== 'audio' && Array.isArray(layer.fx)) {
            for (const effect of layer.fx) ids.add(effect);
          }
          if (layer.type !== 'audio' && Array.isArray(layer.masks) && layer.masks.length) {
            // A single path sends the entire layer's mixed mask list through
            // the Canvas coverage branch; analytic programs are unused there.
            if (layer.masks.some((mask: any) => mask?.path)) maskKeys.add('pathMaskApply');
            else { maskKeys.add('masks'); maskKeys.add('maskApply'); }
          }
        }
        imageSources = null; imageAvailabilityChecked = false;
        effects?.dispose(); effects = undefined; effectStamp = null;
        // Source readiness is independent of the cached structural plan. A
        // bitmap can become available without another animation/project edit.
        plan = staticExportPlan(project, motionBlur,
          layer => typeof layer.d?.asset === 'string' && !!layer.d.asset,
          effect => staticEffectSupported(effect?.type), builtinMask);
        effectIds = plan ? [...ids] : [];
        maskPrograms = plan ? [...maskKeys] : [];
        generation++;
      }
      if (plan && imageLayers.length) {
        media ||= trackExportMedia(PM);
        if (!imageSources || !media.sameSources(imageSources)) {
          const next = media.sources(imageLayers);
          if (imageAvailabilityChecked && (imageSources || next)) generation++;
          imageSources = next; imageAvailabilityChecked = true;
        }
        if (!imageSources) return null;
      }
      const key = fonts?.status === 'loading' ? null : plan?.(time) ?? null;
      if (key !== null && (effectIds.length || maskPrograms.length)) {
        effects ||= trackStaticEffects(PM);
        const next = effects.capture(effectIds, maskPrograms);
        if (next !== effectStamp) { effectStamp = next; generation++; }
        if (!next) return null;
      }
      return key === null ? null : `${generation}:${key}`;
    },
    dispose() {
      if (disposed) return;
      disposed = true;
      media?.dispose(); imageSources = null;
      effects?.dispose(); effectStamp = null;
      for (const event of fontEvents) fonts?.removeEventListener?.(event, invalidate);
      for (const subscription of subscriptions) {
        if (typeof subscription === 'function') subscription();
        else subscription?.dispose?.();
      }
    },
  };
}
