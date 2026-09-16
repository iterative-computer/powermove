function canvasBlob(canvas: HTMLCanvasElement, type: string, quality: number): Promise<Blob | null> {
  if (typeof canvas.toBlob !== 'function') return Promise.resolve(null);
  return new Promise(resolve => {
    try { canvas.toBlob(blob => resolve(blob), type, quality); }
    catch (error) { resolve(null); }
  });
}

async function encodePoster(canvas: HTMLCanvasElement): Promise<Blob | null> {
  return await canvasBlob(canvas, 'image/webp', .8)
    || await canvasBlob(canvas, 'image/jpeg', .8);
}

function posterSize(width: number, height: number, maxEdge: number) {
  if (!(width > 0 && height > 0 && maxEdge > 0)) return null;
  const scale = maxEdge / Math.max(width, height);
  return {
    width: Math.max(1, Math.round(width * scale)),
    height: Math.max(1, Math.round(height * scale)),
  };
}

function seekVideo(video: HTMLVideoElement, time: number, timeout = 2000): Promise<boolean> {
  return new Promise(resolve => {
    let settled = false;
    const finish = (ok: boolean) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      try { video.removeEventListener('seeked', seeked); } catch (error) { }
      resolve(ok);
    };
    const seeked = () => finish(true);
    const timer = setTimeout(() => finish(false), timeout);
    /* Assigning the current position again does not fire `seeked`; a frame is
       already presented there, so draw it without waiting for the timeout. */
    if (Math.abs((Number(video.currentTime) || 0) - time) < .01 && Number(video.readyState) >= 2) {
      finish(true);
      return;
    }
    try {
      video.addEventListener('seeked', seeked);
      video.currentTime = time;
    } catch (error) { finish(false); }
  });
}

export async function capturePoster(asset: any, { maxEdge = 320 }: { maxEdge?: number } = {}): Promise<Blob | null> {
  try {
    if (!asset || (asset.kind !== 'image' && asset.kind !== 'video')) return null;
    const el: any = asset.el;
    if (!el) return null;
    const width = asset.kind === 'video'
      ? Number(el.videoWidth) || 0
      : Number(asset.w || el.naturalWidth || el.width) || 0;
    const height = asset.kind === 'video'
      ? Number(el.videoHeight) || 0
      : Number(asset.h || el.naturalHeight || el.height) || 0;
    const size = posterSize(width, height, Number(maxEdge));
    if (!size) return null;

    const canvas = window.document.createElement('canvas');
    canvas.width = size.width;
    canvas.height = size.height;
    const context = canvas.getContext('2d');
    if (!context || typeof context.drawImage !== 'function') return null;

    if (asset.kind === 'video') {
      const previousTime = Number(el.currentTime) || 0;
      const targetTime = Math.min(.5, Math.max(0, (Number(asset.dur) || 0) * .1));
      try {
        if (!await seekVideo(el, targetTime)) return null;
        context.drawImage(el, 0, 0, size.width, size.height);
      } finally {
        try { el.currentTime = previousTime; } catch (error) { }
      }
    } else context.drawImage(el, 0, 0, size.width, size.height);

    return await encodePoster(canvas);
  } catch (error) { return null; }
}
