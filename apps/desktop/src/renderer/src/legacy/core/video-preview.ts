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
  if (!media?.beginPreview || Math.max(asset.w, asset.h) <= 1920) return Promise.resolve();
  const task = work.catch(() => undefined).then(async () => {
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
        const chunk = await media.readPlaybackProxy(token, offset, Math.min(1024 * 1024, result.size - offset));
        if (!chunk.length) throw new Error('Incomplete video preview');
        chunks.push(new Uint8Array(chunk)); offset += chunk.length;
      }
      const url = URL.createObjectURL(new Blob(chunks, { type: 'video/webm' }));
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
    } finally { await media.releasePlaybackProxy(token).catch(() => undefined); }
  });
  work = task;
  // Preview failure must not turn a successfully imported original into missing media.
  return task.catch(error => { console.warn('Video preview unavailable; using original source.', error); });
}
