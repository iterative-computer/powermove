import type { PMRegistry } from '../registry';

type Sources = { layers: readonly any[]; revision: number; values: unknown[] };
type Prepared = { sources: Sources; time: number; values: unknown[] };
const same = (a: readonly unknown[], b: readonly unknown[]) =>
  a.length === b.length && a.every((value, index) => Object.is(value, b[index]));
const size = (value: number) => Number.isSafeInteger(value) && value > 0;

/** Export-owned freshness checks for immutable image sources and prepared
 * video canvases. Structural eligibility and its cache belong to the caller.
 * Published preparation canvases remain immutable until the next preparation. */
export function trackExportMedia(PM: PMRegistry) {
  let revision = 0, disposed = false;
  const subscription = PM.bus?.on?.('assets', () => { revision++; });

  // Keep one source-field builder for the pre-prepare and ready comparisons.
  // Preparation deliberately changes decoder time/readiness, so those fields
  // are checked only by the ready builder after preparation has completed.
  const sourceValues = (layers: readonly any[]): unknown[] | null => {
    const project = PM.proj;
    const values: unknown[] = [project, project.layers, project.layers.length];
    for (const layer of layers) {
      const id = layer.d?.asset;
      if (!['video', 'image'].includes(layer.type) || typeof id !== 'string' || !id) return null;
      const asset = PM.assets.get(id), element = asset?.el;
      if (!asset || !element || asset.imageSequence || asset.format === 'svg'
          || asset.kind && asset.kind !== layer.type) return null;
      values.push(layer, layer.id, layer.type, layer.d, id, asset, element,
        asset.id, asset.kind, asset.format, asset.url, asset.w, asset.h, asset.dur, asset.videoTimestampScale);
      if (layer.type === 'video') {
        if (typeof HTMLVideoElement === 'undefined' || !(element instanceof HTMLVideoElement)
            || element.srcObject != null || element.error || !size(element.videoWidth) || !size(element.videoHeight)) return null;
        values.push(element.src, element.currentSrc, element.srcObject, element.videoWidth, element.videoHeight);
      } else {
        // HTML images, canvases and SVGs can change pixels in place. A bitmap
        // only changes by replacement or close(), which clears its dimensions.
        if (typeof ImageBitmap === 'undefined' || !(element instanceof ImageBitmap)
            || !size(element.width) || !size(element.height)) return null;
        values.push(element.width, element.height);
      }
    }
    return values;
  };

  const readyValues = (layers: readonly any[], time: number): unknown[] | null => {
    const frames = PM.preparedVideoFrames;
    if (!(frames instanceof Map)) return null;
    const values: unknown[] = [frames, PM.preparedVideoVersion, frames.size];
    for (const [key, canvas] of frames) {
      if (typeof HTMLCanvasElement === 'undefined' || !(canvas instanceof HTMLCanvasElement)
          || !size(canvas.width) || !size(canvas.height)) return null;
      values.push(key, canvas, canvas.width, canvas.height);
    }
    PM.scope.push(PM.proj);
    try {
      for (const layer of layers) {
        if (layer.type !== 'video') continue;
        const active = PM.active(layer, time);
        values.push(layer, active);
        if (!active) continue;
        // The live decoder ends at its last request when several clips share
        // it. Each exact timeline key owns the appropriate earlier snapshot.
        if (!frames.has(`${layer.id}@${time}`)) return null;
        const video = PM.assets.get(layer.d.asset).el as HTMLVideoElement;
        if (video.seeking || video.readyState < 2 || !video.paused || video.error) return null;
        values.push(video.currentTime, video.readyState >= 2, video.seeking, video.paused);
      }
    } finally { PM.scope.pop(); }
    return values;
  };

  const sameSources = (stamp: Sources): boolean => {
    if (disposed || stamp.revision !== revision) return false;
    const values = sourceValues(stamp.layers);
    return !!values && same(stamp.values, values);
  };
  return {
    sources(layers: readonly any[]): Sources | null {
      if (disposed) return null;
      const values = sourceValues(layers);
      return values ? { layers: [...layers], revision, values } : null;
    },
    sameSources,
    prepared(layers: readonly any[], time: number, sources: Sources): Prepared | null {
      if (!same(layers, sources.layers) || !sameSources(sources)) return null;
      const values = readyValues(layers, time);
      return values ? { sources, time, values } : null;
    },
    valid(stamp: Prepared): boolean {
      if (!sameSources(stamp.sources)) return false;
      const values = readyValues(stamp.sources.layers, stamp.time);
      return !!values && same(stamp.values, values);
    },
    dispose() {
      if (disposed) return;
      disposed = true;
      if (typeof subscription === 'function') subscription();
      else subscription?.dispose?.();
    },
  };
}
