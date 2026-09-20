import { cancelPreviewVideoSeek } from './video-seek';

/** Full preview resolution and offline rendering always use the source. */
export function previewVideoElement(PM: any, asset: any): HTMLVideoElement {
  return asset.preview?.el && (PM.perf?.auto || PM.quality < 1)
    && !PM.Export?.busy && !PM.agentFrameCapture && !PM.Preview?.preparing
    ? asset.preview.el : asset.el;
}

let work: Promise<unknown> = Promise.resolve();

/** Build disposable editing media independently of the portable original. */
export function prepareVideoPreview(PM: any, asset: any, source: Blob, disposed: () => boolean): Promise<void> {
  const media = window.powermove?.media;
  const legacyCutout = asset.playbackProxy && Number(asset.playbackProxyVersion || 0) < 3;
  if (!media?.beginPreview || (!legacyCutout && Math.max(asset.w, asset.h) <= 1920)) return Promise.resolve();
  // Derivatives have their own versioned key; never replace the portable source.
  const cacheKey = asset.storageKey
    ? `${asset.storageKey}:preview:intra-1280-v1:${asset.playbackProxyVersion || 0}:${source.size}` : null;
  const installPreview = async (blob: Blob) => {
    const url = URL.createObjectURL(blob);
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
      if (disposed()) { el.removeAttribute('src'); el.load(); URL.revokeObjectURL(url); return; }
      asset.preview = { el, url };
      cancelPreviewVideoSeek(asset.el); asset.el.pause();
      PM.bus.emit('assets'); PM.invalidate('render');
    } catch (error) { el.removeAttribute('src'); el.load(); URL.revokeObjectURL(url); throw error; }
  };
  // A cached preview bypasses the conversion queue entirely. Older alpha clips
  // also bypass optional 4K work when they need their first conversion.
  const task = Promise.resolve().then(async () => {
    if (disposed()) return;
    if (cacheKey) {
      const cached = await Promise.resolve(PM.MediaStore.get(cacheKey)).catch(() => null);
      if (disposed()) return;
      if (cached) {
        try { await installPreview(cached); return; }
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
        await installPreview(blob);
        if (cacheKey && !disposed()) {
          // Cache failure is harmless: the portable source remains authoritative.
          await PM.MediaStore.put(cacheKey, blob, { storageKey: cacheKey }).catch(() => false);
        }
      } finally { await media.releasePlaybackProxy(token).catch(() => undefined); }
    };
    const conversion = legacyCutout ? convert() : work.catch(() => undefined).then(convert);
    if (!legacyCutout) work = conversion;
    await conversion;
  });
  // Preview failure must not turn a successfully imported original into missing media.
  return task.catch(error => { console.warn('Video preview unavailable; using original source.', error); });
}
