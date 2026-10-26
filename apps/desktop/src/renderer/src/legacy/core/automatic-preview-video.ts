import { videoClipsAt } from './video-timeline';
import { previewVideoElement } from './video-preview';

type Decoder = { source: HTMLVideoElement; url: string; video: HTMLVideoElement; at: number | null };
type PreviewVideoRequest = { key: string; asset: any; at: number; exact?: boolean };
const MAX_DECODERS = 4;
const WAIT_MS = 2000;
const TIME_EPSILON = .0005;

/** Idle preparation has its own decoders. Seeking the editing decoder here
 * would disturb the visible canvas, playback and the next press of Play. */
export function createAutomaticPreviewVideos(PM: any): {
  prepare(time: number, valid: () => boolean, options?: {mblur?: boolean; mbSamples?: number; shutter?: number}): Promise<Map<string, ImageBitmap> | null>;
  release(frames: Map<string, ImageBitmap>): void;
  readonly bytes: number;
  readonly error: string | null;
  cancel(): void;
  dispose(): void;
} {
  const decoders = new Map<HTMLVideoElement, Decoder>();
  const owned = new Map<Map<string, ImageBitmap>, { bytes: number }>();
  let capturedBytes = 0;
  let lastError: string | null = null;
  let active: AbortController | null = null;
  let disposed = false;
  let inactivity: ReturnType<typeof setTimeout> | null = null;

  const clearInactivity = () => {
    if (inactivity !== null) clearTimeout(inactivity);
    inactivity = null;
  };

  const release = (decoder: Decoder) => {
    try { decoder.video.pause(); } catch { /* A failed private decoder is disposable. */ }
    try { decoder.video.removeAttribute('src'); decoder.video.load(); } catch { /* Detached media. */ }
  };
  const releaseAll = () => {
    clearInactivity();
    for (const decoder of decoders.values()) release(decoder);
    decoders.clear();
  };
  const releaseFrames = (frames: Map<string, ImageBitmap>) => {
    const state = owned.get(frames);
    if (!state) return;
    owned.delete(frames); capturedBytes -= state.bytes;
    for (const frame of frames.values()) frame.close();
    frames.clear();
  };
  const cancel = () => {
    clearInactivity();
    active?.abort(); active = null;
    releaseAll();
    for (const frames of owned.keys()) releaseFrames(frames);
  };

  async function prepare(time: number, valid: () => boolean, options?: {mblur?: boolean; mbSamples?: number; shutter?: number}): Promise<Map<string, ImageBitmap> | null> {
    if (disposed || !Number.isFinite(time)) return null;
    clearInactivity();
    lastError = null;
    // Concurrent requests cannot seek the same private decoder. A new request
    // supersedes unfinished work; already returned bitmaps belong to the caller.
    if (active) cancel();
    const controller = new AbortController();
    active = controller;
    const frames = new Map<string, ImageBitmap>();
    const captureState = { bytes: 0 };
    owned.set(frames, captureState);
    const current = () => !disposed && !controller.signal.aborted && valid();
    let completed = false;
    try {
      if (!current()) throw new Error('Preview preparation cancelled');
      let planned: PreviewVideoRequest[];
      try {
        // The compositor also renders forced-on track mattes, group passes and
        // solo precomps. The ordinary playback clip list does not include all
        // of those sources and cannot safely plan a finished preview for them.
        planned = options && typeof PM.GL?.frameVideoRequests === 'function'
          ? PM.GL.frameVideoRequests(PM.proj,time,options).map((request:any)=>({...request,exact:true}))
          : typeof PM.GL?.thumbnailVideoRequests === 'function'
          ? PM.GL.thumbnailVideoRequests(PM.proj, time)
          : videoClipsAt(PM, time).map(clip => ({ key: `${clip.id}@${clip.time}`, asset: clip.asset, at: clip.at }));
      } finally {
        // Collection evaluates the future frame synchronously. Restore the
        // editor's evaluation context before any loading or decoding can yield.
        PM.beginEval?.(PM.time);
      }
      const unique = new Map<string, PreviewVideoRequest>();
      for (const request of planned) {
        if (!request || typeof request.key !== 'string' || !request.key) throw new Error('Invalid preview video request');
        const existing = unique.get(request.key);
        if (existing && (existing.asset !== request.asset || existing.at !== request.at)) {
          throw new Error('Conflicting preview video instances');
        }
        if (!existing) unique.set(request.key, request);
      }
      // Copy the descriptor: source selection must not clear a deferred proxy
      // flag on the live asset as a side effect of speculative preparation.
      const requests = [...unique.values()].map(request => ({ request, source: previewVideoElement(PM, {
        ...request.asset, preview: request.asset.preview ? { ...request.asset.preview } : undefined,
      }) }));
      const sources = new Set(requests.map(request => request.source));
      for (const [source, decoder] of decoders) if (!sources.has(source)) {
        release(decoder); decoders.delete(source);
      }
      if (requests.length && (typeof VideoFrame !== 'function' || typeof createImageBitmap !== 'function')) {
        throw new Error('Decoded frame capture is unavailable');
      }
      for (const { request, source } of requests) {
        if (!current() || !source || source.srcObject || !Number.isFinite(request.at)) throw new Error('Unsafe preview source');
        const url = source.currentSrc || source.src;
        if (!url) throw new Error('Missing preview source');
        let decoder = decoders.get(source);
        if (decoder && decoder.url !== url) { release(decoder); decoders.delete(source); decoder = undefined; }
        if (!decoder) {
          while (decoders.size >= MAX_DECODERS) {
            const [oldSource, old] = decoders.entries().next().value!;
            release(old); decoders.delete(oldSource);
          }
          const video = document.createElement('video');
          video.muted = true; video.playsInline = true; video.preload = 'auto';
          if (source.crossOrigin !== null) video.crossOrigin = source.crossOrigin;
          if (typeof video.requestVideoFrameCallback !== 'function' || typeof video.cancelVideoFrameCallback !== 'function') {
            release({ source, url, video, at: null });
            throw new Error('Presented frame callbacks are unavailable');
          }
          decoder = { source, url, video, at: null };
          decoders.set(source, decoder);
          await loaded(video, url, controller.signal);
        } else {
          // LRU order keeps the active sources nearest the current scene.
          decoders.delete(source); decoders.set(source, decoder);
        }
        if (!current()) throw new Error('Preview preparation cancelled');
        // Frozen source images are temporary preview memory too. Reserve their
        // space before snapshotting; do not consume the final frame's space or
        // let a collection of 4K clips exceed the user's shared preview limit.
        const size = decoder.video.videoWidth * decoder.video.videoHeight * 4;
        const width = Number(PM.GL?.canvas?.width ?? Math.ceil(Number(PM.proj.w || 1) * Number(PM.quality || 1)));
        const height = Number(PM.GL?.canvas?.height ?? Math.ceil(Number(PM.proj.h || 1) * Number(PM.quality || 1)));
        const available = Math.max(0, (PM.Memory?.budget?.('preview') ?? 256 * 1024 * 1024)
          - (PM.GL?.previewFrames?.bytes || 0) - (PM.Preview?.bytes || 0)
          - (options ? PM.AutomaticPreview?.stagingBytes || 0 : PM.GL?.inputPreviewBytes || 0) - width * height * 4);
        // Exact shutter invocations can need more than one ordinary idle
        // snapshot (for example twelve full-HD samples). Bound them by both
        // the texture allowance and the shared preview cap, not the idle ceiling.
        const limit = Math.min(options ? PM.Memory?.budget?.('textures') ?? 128 * 1024 * 1024 : 64 * 1024 * 1024, available);
        if (!Number.isSafeInteger(size) || size <= 0 || !Number.isFinite(limit) || capturedBytes + size > limit) {
          throw new Error('Video preview exceeds available memory');
        }
        captureState.bytes += size; capturedBytes += size;
        const snapshot = await decodedFrame(decoder, request, controller.signal);
        let bitmap: ImageBitmap;
        try {
          if (!decoder.video.paused || decoder.video.seeking || decoder.video.readyState < 2) {
            throw new Error('Private video frame is no longer stable');
          }
          // Preserve the browser's normal DOM-video color/alpha conversion.
          // The VideoFrame validates its timestamp; this private decoder stays
          // paused until its pixels have been frozen or the job is cancelled.
          bitmap = await freeze(decoder.video, controller.signal);
        }
        finally { snapshot.close(); }
        if (!current() || !owned.has(frames)) { bitmap.close(); throw new Error('Preview preparation cancelled'); }
        // The compositor's isolated capture mode addresses repeated precomps
        // by their complete instance path, with the nested composition time.
        const key = request.key;
        if (frames.has(key)) { bitmap.close(); throw new Error('Ambiguous preview video instance'); }
        frames.set(key, bitmap);
      }
      if (!current()) throw new Error('Preview preparation cancelled');
      completed = true;
      return frames;
    } catch (error) {
      lastError = error instanceof Error ? error.message : String(error);
      releaseFrames(frames);
      if (active === controller) releaseAll();
      return null;
    } finally {
      if (active === controller) {
        active = null;
        // Keep codecs across a short sequence of adjacent frames, then unload
        // them after warming ends. Frozen images do not depend on the decoder.
        if (completed && decoders.size) inactivity = setTimeout(() => {
          inactivity = null;
          if (!active) releaseAll();
        }, WAIT_MS);
      }
    }
  }

  return { prepare, cancel, release: releaseFrames, get bytes() { return capturedBytes; }, get error() { return lastError; }, dispose() { disposed = true; cancel(); } };
}

function loaded(video: HTMLVideoElement, url: string, signal: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    const ready = () => { if (video.readyState >= 2 && video.videoWidth > 0 && video.videoHeight > 0) finish(); };
    const failed = () => finish(new Error('Video preview could not load'));
    const aborted = () => finish(new Error('Video preview cancelled'));
    const timer = setTimeout(failed, WAIT_MS);
    const finish = (error?: Error) => {
      clearTimeout(timer);
      video.removeEventListener('loadeddata', ready); video.removeEventListener('error', failed);
      signal.removeEventListener('abort', aborted);
      error ? reject(error) : resolve();
    };
    video.addEventListener('loadeddata', ready); video.addEventListener('error', failed);
    signal.addEventListener('abort', aborted, { once: true });
    if (signal.aborted) { aborted(); return; }
    try { video.src = url; ready(); } catch { failed(); }
  });
}

/** readyState/currentTime alone can describe an earlier frame after a seek.
 * Require presentation and keep the actual decoded timestamp with its pixels. */
function decodedFrame(decoder: Decoder, request: PreviewVideoRequest, signal: AbortSignal): Promise<VideoFrame> {
  const video = decoder.video;
  const at = request.at, epsilon = request.exact ? .0000011 : TIME_EPSILON;
  let observed = 'no presentation';
  const capture = (presentedTime?: number): VideoFrame | null => {
    if (video.seeking || video.readyState < 2 || !video.videoWidth || !video.videoHeight) return null;
    let frame: VideoFrame;
    try { frame = new VideoFrame(video); } catch { return null; }
    const start = frame.timestamp / 1e6;
    const fps = Number(request.asset.imageSequence?.fps || request.asset.fps);
    const duration = Number(frame.duration) > 0 ? Number(frame.duration) / 1e6
      : Number.isFinite(fps) && fps > 0 ? 1 / fps : 0;
    observed = `frame ${start}, duration ${duration}, presentation ${presentedTime}, seek ${video.currentTime}`;
    const presented = presentedTime === undefined || Math.abs(start - presentedTime) < TIME_EPSILON;
    const coversTarget = Math.abs(start - at) < epsilon
      || start <= at + epsilon && duration > 0 && at < start + duration - epsilon;
    if (presented && coversTarget) return frame;
    frame.close(); return null;
  };
  if (decoder.at === at && video.paused && !signal.aborted) {
    const frame = capture();
    if (frame) return Promise.resolve(frame);
  }
  return new Promise((resolve, reject) => {
    let finished = false;
    let callback = 0;
    let seekCompleted = false;
    let playing = false;
    const cleanup = () => {
      clearTimeout(timer);
      video.removeEventListener('seeked', ready); video.removeEventListener('loadeddata', ready);
      video.removeEventListener('error', failed); signal.removeEventListener('abort', aborted);
      if (callback) video.cancelVideoFrameCallback(callback);
      try { video.pause(); } catch { /* Private media can be discarded. */ }
    };
    const finish = (frame: VideoFrame | null) => {
      if (finished) { frame?.close(); return; }
      finished = true; cleanup();
      if (frame) { decoder.at = at; resolve(frame); }
      else reject(new Error(`Video preview did not decode ${at}: ${observed}`));
    };
    const failed = () => finish(null);
    const aborted = () => finish(null);
    const ready = () => {
      if (finished || video.seeking || video.readyState < 2 || Math.abs(video.currentTime - at) >= epsilon) return;
      seekCompleted = true;
      // A seeked private decoder may already expose a timestamp-validated
      // frame. Freeze it before playback can skip a high-frame-rate target.
      if (request.exact) { const frame = capture(); if (frame) { finish(frame); return; } }
      // Detached decoders may not present a paused frame. Play only this muted
      // private element until its first matching presentation, then pause it.
      if (!playing) {
        playing = true;
        try { Promise.resolve(video.play()).catch(failed); } catch { failed(); }
      }
    };
    const presented: VideoFrameRequestCallback = (_now, metadata) => {
      callback = 0;
      if (finished) return;
      const frame = seekCompleted ? capture(metadata.mediaTime) : null;
      if (frame) { finish(frame); return; }
      try { callback = video.requestVideoFrameCallback(presented); } catch { failed(); }
    };
    const timer = setTimeout(failed, WAIT_MS);
    video.addEventListener('seeked', ready); video.addEventListener('loadeddata', ready);
    video.addEventListener('error', failed); signal.addEventListener('abort', aborted, { once: true });
    if (signal.aborted) { aborted(); return; }
    try {
      video.pause(); decoder.at = null;
      callback = video.requestVideoFrameCallback(presented);
      video.currentTime = at; ready();
    } catch { failed(); }
  });
}

/** ImageBitmap creation is asynchronous and cannot be cancelled natively.
 * Close late results after cancellation instead of publishing or leaking them. */
function freeze(video: HTMLVideoElement, signal: AbortSignal): Promise<ImageBitmap> {
  return new Promise((resolve, reject) => {
    let finished = false;
    const cleanup = () => { clearTimeout(timer); signal.removeEventListener('abort', failed); };
    const failed = () => {
      if (finished) return;
      finished = true; cleanup(); reject(new Error('Video preview capture cancelled'));
    };
    const timer = setTimeout(failed, WAIT_MS);
    signal.addEventListener('abort', failed, { once: true });
    if (signal.aborted) { failed(); return; }
    try {
      // WebGL ignores UNPACK_PREMULTIPLY_ALPHA_WEBGL for ImageBitmaps. Match
      // the compositor's normal video upload explicitly, including alpha clips.
      Promise.resolve(createImageBitmap(video, { premultiplyAlpha: 'premultiply' })).then(bitmap => {
        if (finished) { bitmap.close(); return; }
        finished = true; cleanup(); resolve(bitmap);
      }, failed);
    } catch { failed(); }
  });
}
