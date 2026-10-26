export class VideoFrameDecodeError extends Error {
  constructor(message = 'Video frame decode timed out') { super(message); this.name = 'VideoFrameDecodeError'; }
}

export function captureAborted(signal?: AbortSignal): void {
  if (signal?.aborted) throw signal.reason ?? new DOMException('Frame capture cancelled', 'AbortError');
}

/** Metadata/seek completion and decoded pixels are separate readiness states.
 * A seek to the initial time need not emit seeked at all. */
export function seekVideoFrame(video: HTMLVideoElement, at: number,
  options: { signal?: AbortSignal; timeoutMs?: number; tolerance?: number } = {}): Promise<void> {
  return new Promise((resolve, reject) => {
    const { signal } = options;
    // Chromium exposes a microsecond media clock, including fractional FPS.
    const tolerance = Math.max(.0000011, options.tolerance ?? .0005);
    let target = at;
    let settled = false, requested = false;
    const events = ['loadedmetadata', 'loadeddata', 'canplay', 'seeked'];
    const cleanup = () => {
      clearTimeout(timer); clearInterval(poll);
      for (const event of events) video.removeEventListener(event, check);
      video.removeEventListener('error', failed);
      signal?.removeEventListener('abort', aborted);
    };
    const finish = (error?: unknown) => {
      if (settled) return;
      settled = true; cleanup();
      error ? reject(error) : resolve();
    };
    const aborted = () => finish(signal?.reason ?? new DOMException('Frame capture cancelled', 'AbortError'));
    const diagnostic = () => `source ${target.toFixed(6)}s; current ${Number(video.currentTime).toFixed(6)}s; readyState ${video.readyState}; seeking ${video.seeking}; media error ${video.error?.code ?? 'none'}`;
    const failed = () => finish(new VideoFrameDecodeError(`Could not decode video frame (${diagnostic()})`));
    const check = () => {
      if (settled) return;
      if (signal?.aborted) return aborted();
      if (video.error) return failed();
      // Assigning currentTime before metadata can fail or lose the requested seek.
      if (video.readyState < 1) return;
      try {
        // Project metadata can outlive a relink/conversion. The element's
        // duration is authoritative; seeking to the end yields no next frame.
        target = Number.isFinite(video.duration) && video.duration > 0
          ? Math.min(at, Math.max(0, video.duration - .000001)) : at;
        if (!Number.isFinite(target) || target < 0) {
          finish(new VideoFrameDecodeError('Could not decode video frame: invalid source time')); return;
        }
        if (!requested) {
          requested = true;
          if (video.seeking || Math.abs(video.currentTime - target) >= tolerance) video.currentTime = target;
        }
        if (!video.seeking && video.readyState >= 2 && Math.abs(video.currentTime - target) < tolerance) finish();
      } catch (error) { finish(error); }
    };
    const timer = setTimeout(() => {
      check();
      if (!settled) finish(new VideoFrameDecodeError(`Video frame decode timed out after ${options.timeoutMs ?? 10_000}ms (${diagnostic()})`));
    }, options.timeoutMs ?? 10_000);
    // Some decoders update readiness without another loadeddata/canplay event.
    // Observe only: repeatedly assigning currentTime would restart decoding.
    const poll = setInterval(check, 100);
    for (const event of events) video.addEventListener(event, check);
    video.addEventListener('error', failed);
    signal?.addEventListener('abort', aborted, { once: true });
    video.pause();
    check();
  });
}

export function withCaptureSignal<T>(promise: PromiseLike<T>, signal?: AbortSignal): Promise<T> {
  if (!signal) return Promise.resolve(promise);
  return new Promise((resolve, reject) => {
    const abort = () => reject(signal.reason ?? new DOMException('Frame capture cancelled', 'AbortError'));
    if (signal.aborted) { abort(); return; }
    signal.addEventListener('abort', abort, { once: true });
    Promise.resolve(promise).then(resolve, reject).finally(() => signal.removeEventListener('abort', abort));
  });
}
