type Request = { key: string; asset: any; at: number };
type CapturedFrame = VideoFrame | HTMLCanvasElement;
export interface ThumbnailVideoFrames {
  frames: Map<string, CapturedFrame>;
  version: string;
  dispose(): void;
}

let queue = Promise.resolve();
let version = 0;
const cancelled = () => Object.assign(new Error('Composition thumbnail cancelled'), { name: 'AbortError' });

function waitFor(video: HTMLVideoElement, signal: AbortSignal, ready: () => boolean, start: () => void): Promise<void> {
  return new Promise((resolve, reject) => {
    const cleanup = () => {
      clearTimeout(timer);
      for (const event of ['loadeddata', 'seeked']) video.removeEventListener(event, done);
      video.removeEventListener('error', failed); signal.removeEventListener('abort', aborted);
    };
    const done = () => { if (ready()) { cleanup(); resolve(); } };
    const failed = () => { cleanup(); reject(new Error('Could not decode composition thumbnail video')); };
    const aborted = () => { cleanup(); reject(cancelled()); };
    const timer = setTimeout(() => { cleanup(); reject(new Error('Composition thumbnail video decode timed out')); }, 15_000);
    for (const event of ['loadeddata', 'seeked']) video.addEventListener(event, done);
    video.addEventListener('error', failed); signal.addEventListener('abort', aborted, { once: true });
    if (signal.aborted) { aborted(); return; }
    try { start(); done(); } catch (error) { cleanup(); reject(error); }
  });
}

/** A single private decoder serves captures serially; the viewer's decoders,
 * seeks and prepared export frames retain their owners throughout the wait. */
export function prepareThumbnailVideoFrames(PM: any, comp: any, time: number, signal: AbortSignal | (() => AbortSignal)): Promise<ThumbnailVideoFrames> | null {
  const requests: Request[] = PM.GL?.thumbnailVideoRequests?.(comp, time) ?? [];
  if (!requests.length) return null;
  const abortSignal = typeof signal === 'function' ? signal() : signal;
  const task = queue.then(async () => {
    const frames = new Map<string, CapturedFrame>();
    const copied = new Map<any, Map<number, CapturedFrame>>();
    let video: HTMLVideoElement | null = null, source = '';
    const dispose = () => {
      for (const variants of copied.values()) for (const frame of variants.values()) {
        if ('close' in frame) frame.close(); else { frame.width = 0; frame.height = 0; }
      }
      frames.clear(); copied.clear();
    };
    // These are temporary native references for this one composition, not a
    // cache. A soft texture-cache budget must not reject an otherwise supported
    // composition. The serial decoder, request deduplication and immediate
    // frame/upload disposal bound ownership without lowering source quality.
    try {
      if (abortSignal.aborted) throw cancelled();
      for (const request of requests) {
        if (abortSignal.aborted) throw cancelled();
        const existing = copied.get(request.asset)?.get(request.at);
        if (existing) { frames.set(request.key, existing); continue; }
        video ??= document.createElement('video');
        video.muted = true; video.playsInline = true; video.preload = 'auto';
        const nextSource = request.asset.el.currentSrc || request.asset.el.src;
        if (!nextSource) throw new Error('Composition thumbnail video source is unavailable');
        if (source !== nextSource) {
          source = nextSource;
          await waitFor(video, abortSignal, () => video!.currentSrc === nextSource && video!.readyState >= 2 && !!video!.videoWidth && !!video!.videoHeight,
            () => { video!.src = nextSource; });
        }
        await waitFor(video, abortSignal, () => !video!.seeking && video!.readyState >= 2 && Math.abs(video!.currentTime - request.at) < .0005,
          () => { if (Math.abs(video!.currentTime - request.at) >= .0005 || video!.seeking) video!.currentTime = request.at; });
        if (abortSignal.aborted) throw cancelled();
        const width = video.videoWidth, height = video.videoHeight;
        let variants = copied.get(request.asset);
        if (!variants) copied.set(request.asset, variants = new Map());
        let frame: CapturedFrame;
        if (typeof VideoFrame !== 'undefined') {
          try { frame = new VideoFrame(video); }
          catch { frame = document.createElement('canvas'); }
        } else frame = document.createElement('canvas');
        variants.set(request.at, frame);
        if (!('close' in frame)) {
          frame.width = width; frame.height = height;
          const context = frame.getContext('2d');
          if (!context) throw new Error('Could not copy composition thumbnail video frame');
          context.globalCompositeOperation = 'copy'; context.drawImage(video, 0, 0);
        }
        frames.set(request.key, frame);
      }
      return { frames, version: 'thumbnail:' + ++version, dispose };
    } catch (error) { dispose(); throw error; }
    finally {
      if (video) { video.pause(); video.removeAttribute('src'); video.load(); }
    }
  });
  queue = task.then(() => {}, () => {});
  return task;
}
