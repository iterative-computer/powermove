import { previewVideoElement } from './video-preview';
import { cancelPreviewVideoSeek } from './video-seek';
import { stopPlaybackVideoFrames } from './video-playback-frames';

type Instance = { el: HTMLVideoElement; release: () => void };
type Pool = { source: HTMLVideoElement; layers: Map<string, Instance> };
const pools = new WeakMap<object, Pool>();
const textureIds = new WeakMap<object, number>();
let nextTextureId = 0;

/** Each clip owns its decoder time, even when clips reference the same file. */
export function layerVideoElement(PM: any, asset: any, layerId: string): HTMLVideoElement {
  const source = previewVideoElement(PM, asset);
  let pool = pools.get(asset);
  if (pool && pool.source !== source) {
    disposeVideoInstances(asset);
    pool = undefined;
  }
  if (!pool) {
    pool = { source, layers: new Map() };
    pools.set(asset, pool);
  }
  const existing = pool.layers.get(layerId);
  if (existing) return existing.el;
  // Reuse the imported decoder for the first clip; only copies need another.
  const ownedSource = [...pool.layers.values()].some(instance => instance.el === source);
  const el = ownedSource ? document.createElement('video') : source;
  const ready = () => PM.invalidate('render');
  if (el !== source) {
    el.muted = true; el.playsInline = true; el.preload = 'auto';
    el.addEventListener('loadeddata', ready);
    el.src = source.currentSrc || source.src;
  }
  pool.layers.set(layerId, { el, release: () => {
    stopPlaybackVideoFrames(el);
    cancelPreviewVideoSeek(el);
    el.pause();
    if (el !== source) {
      el.removeEventListener('loadeddata', ready);
      el.removeAttribute('src'); el.load();
    }
  } });
  return el;
}

/** Decoder replacements must never reuse an older element's texture version. */
export function videoInstanceTextureKey(el: HTMLVideoElement): string {
  if (!textureIds.has(el)) textureIds.set(el, ++nextTextureId);
  return `video:${textureIds.get(el)}`;
}

export function pauseVideoInstances(asset: object): void {
  for (const { el } of pools.get(asset)?.layers.values() ?? []) {
    stopPlaybackVideoFrames(el);
    cancelPreviewVideoSeek(el); el.pause();
  }
}

export function pruneVideoInstances(asset: object, layerIds: ReadonlySet<string>, exact = false): void {
  const pool = pools.get(asset);
  if (!pool) return;
  for (const [id, instance] of pool.layers) {
    if (layerIds.has(id) || (!exact && layerIds.has(id.slice(id.lastIndexOf('/') + 1)))) continue;
    instance.release(); pool.layers.delete(id);
  }
  if (!pool.layers.size) pools.delete(asset);
}

export function disposeVideoInstances(asset: object): void {
  const pool = pools.get(asset);
  if (!pool) return;
  for (const instance of pool.layers.values()) instance.release();
  pools.delete(asset);
}
