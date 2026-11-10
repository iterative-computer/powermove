import { cancelPreviewVideoSeek } from './video-seek';
import { bridge as hostBridge } from '../../kernel/bridge';
import { isStreamedMedia, type StreamedMedia } from '../../host/streamed-media';
import { trackVideo } from './video-residency';

type VideoDemand={width:number;height:number};
const materialDemands=new WeakMap<object,WeakMap<object,VideoDemand|false>>();
/** A material-only clip can decode editing media when it supplies every texel
 * the visible surface needs. Ordinary footage keeps its chosen preview mode. */
export function setMaterialVideoDemands(PM:any,clips:Iterable<{asset:any;id:string;materialSize?:VideoDemand}>):void {
  const demands=new WeakMap<object,VideoDemand|false>();
  for(const clip of clips){
    if(!clip.materialSize){demands.set(clip.asset,false);continue;}
    const previous=demands.get(clip.asset);if(previous===false)continue;
    const size=PM.GL?.materialVideoSize?.(clip.id) ?? clip.materialSize;
    demands.set(clip.asset,{width:Math.max(previous?.width || 0,size.width),height:Math.max(previous?.height || 0,size.height)});
  }
  materialDemands.set(PM,demands);
}

/** Offline rendering always uses the portable original. */
export function previewVideoElement(PM: any, asset: any): HTMLVideoElement {
  // A derivative that finishes during playback joins on the next pause.
  // Replacing a running decoder would interrupt an otherwise smooth clip.
  if (asset.preview?.deferred) {
    if (PM.playing) return asset.el;
    asset.preview.deferred = false;
  }
  const demand=materialDemands.get(PM)?.get(asset),preview=asset.preview?.el;
  const enough=demand && preview?.videoWidth>=demand.width && preview?.videoHeight>=demand.height;
  return preview && (demand?enough:PM.perf?.auto || PM.quality < 1)
    && !PM.Export?.busy && !PM.agentFrameCapture && !PM.Preview?.preparing
    ? asset.preview.el : asset.el;
}

let work: Promise<unknown> = Promise.resolve();

/* Opening a project installs every stored preview. Each install loads a
   player and decodes a frame, so doing all of them at once spiked memory by
   more than a gigabyte on an 87-clip edit. A couple at a time is just as fast
   in practice; the residency pass then releases the ones nobody is viewing. */
const INSTALL_SLOTS = 2;
let installing = 0;
const installQueue: Array<() => void> = [];
async function installSlot<T>(run: () => Promise<T>): Promise<T> {
  if (installing >= INSTALL_SLOTS) await new Promise<void>(resolve => installQueue.push(resolve));
  installing++;
  try { return await run(); }
  finally { installing--; installQueue.shift()?.(); }
}

/** Build disposable editing media independently of the portable original. */
export function prepareVideoPreview(PM: any, asset: any, source: Blob | StreamedMedia, disposed: () => boolean): Promise<void> {
  const media = hostBridge()?.media;
  const legacyCutout = asset.playbackProxy && Number(asset.playbackProxyVersion || 0) < 3;
  // Long GOPs make HD footage expensive to seek too. Build editing media for
  // ordinary videos regardless of size. Large sequences still need smaller
  // editing copies: their frame grid does not make full-resolution decoding cheap.
  const smallSequence = asset.imageSequence && Math.max(asset.w, asset.h) <= 1920;
  if (!media?.beginPreview || (!legacyCutout && smallSequence)) return Promise.resolve();
  // Derivatives have their own versioned key; never replace the portable source.
  const cacheKey = asset.storageKey
    ? `${asset.storageKey}:preview:intra-1280-v2:${asset.playbackProxyVersion || 0}:${source.size}` : null;
  // A stored derivative plays straight from the library file; only a freshly
  // converted one is a Blob until it has been saved.
  const installPreview = async (blob: Blob | StreamedMedia) => {
    const url = isStreamedMedia(blob) ? blob.url : URL.createObjectURL(blob);
    const release = () => { if (!isStreamedMedia(blob)) URL.revokeObjectURL(url); };
    const el = document.createElement('video'); el.muted = true; el.playsInline = true; el.preload = 'auto';
    try {
      await new Promise<void>((resolve, reject) => {
        const timer = setTimeout(() => done(new Error('Video preview timed out')), 15000);
        const loaded = () => done(), failed = () => done(new Error('Could not decode video preview'));
        const done = (error?: Error) => {
          clearTimeout(timer); el.removeEventListener('loadeddata', loaded); el.removeEventListener('error', failed);
          error ? reject(error) : resolve();
        };
        el.addEventListener('loadeddata', loaded); el.addEventListener('error', failed); el.src = url;
      });
      // Chromium can fire loadeddata before a detached video's first frame
      // can be copied into a texture. Wait for presentation before publishing it.
      await new Promise<void>((resolve, reject) => {
        const timer = setTimeout(() => { el.pause(); reject(new Error('Video preview did not present a frame')); }, 5000);
        const ready = () => { clearTimeout(timer); el.pause(); resolve(); };
        if (el.requestVideoFrameCallback) el.requestVideoFrameCallback(ready);
        else ready();
        void el.play().catch(error => { clearTimeout(timer); reject(error); });
      });
      if (disposed()) { el.removeAttribute('src'); el.load(); release(); return; }
      asset.preview = { el, url, deferred: !!PM.playing };
      // Not in use yet: the next residency pass may release its decoder.
      trackVideo(el, -Infinity);
      if (previewVideoElement(PM, asset) === el) {
        cancelPreviewVideoSeek(asset.el); asset.el.pause();
      }
      PM.bus.emit('assets', { runtimeOnly: true }); PM.invalidate('render');
    } catch (error) { el.removeAttribute('src'); el.load(); release(); throw error; }
  };
  // A cached preview bypasses the conversion queue entirely. Older alpha clips
  // also bypass optional 4K work when they need their first conversion.
  const task = Promise.resolve().then(async () => {
    if (disposed()) return;
    if (cacheKey) {
      const cached = await Promise.resolve(PM.MediaStore.source?.(cacheKey) ?? null).catch(() => null)
        ?? await Promise.resolve(PM.MediaStore.get(cacheKey)).catch(() => null);
      if (disposed()) return;
      if (cached) {
        try { await installSlot(() => installPreview(cached)); return; }
        catch { /* A damaged derivative can be rebuilt from the original. */ }
      }
    }
    const convert = async () => {
      if (disposed()) return;
      const token = await media.beginPreview(source.size);
      try {
        for (let offset = 0; offset < source.size; offset += 1024 * 1024) {
          if (disposed()) return;
          await media.writePreview(token, offset, new Uint8Array(await source.slice(offset, offset + 1024 * 1024).arrayBuffer()));
        }
        const result = await media.finishPreview(token);
        if (disposed()) return;
        const chunks: Uint8Array<ArrayBuffer>[] = [];
        for (let offset = 0; offset < result.size;) {
          if (disposed()) return;
          const chunk = await media.readPlaybackProxy(token, offset, Math.min(1024 * 1024, result.size - offset));
          if (disposed()) return;
          if (!chunk.length) throw new Error('Incomplete video preview');
          chunks.push(new Uint8Array(chunk)); offset += chunk.length;
        }
        const blob = new Blob(chunks, { type: 'video/webm' });
        chunks.length = 0;
        // Store first, then play the stored file. Playing the Blob kept every
        // finished preview in memory (about 10 MB per clip) until the project
        // closed. Cache failure is harmless: the Blob still plays.
        const stored = cacheKey && !disposed()
          ? await PM.MediaStore.put(cacheKey, blob, { storageKey: cacheKey }).catch(() => false) : false;
        const streamed = stored && !disposed()
          ? await Promise.resolve(PM.MediaStore.source?.(cacheKey) ?? null).catch(() => null) : null;
        await installPreview(streamed ?? blob);
      } finally { await media.releasePlaybackProxy(token).catch(() => undefined); }
    };
    const conversion = legacyCutout ? convert() : work.catch(() => undefined).then(convert);
    if (!legacyCutout) work = conversion;
    await conversion;
  });
  // Preview failure must not turn a successfully imported original into missing media.
  return task.catch(error => { console.warn('Video preview unavailable; using original source.', error); });
}
