type SeekFrame = { canvas: HTMLCanvasElement; time: number; version: number };
type SeekState = { target: number; tolerance: number; active: boolean; frame?: SeekFrame; version: number };
const pending = new WeakMap<HTMLVideoElement, SeekState>();

/** Keep the newest scrub request without repeatedly flushing an in-flight decode. */
export function seekPreviewVideo(video: HTMLVideoElement, target: number, tolerance: number): void {
  let state = pending.get(video);
  if (!state) {
    state = { target, tolerance, active: true, version: 0 };
    pending.set(video, state);
    const resume = () => {
      if (state!.active && video.paused && !video.seeking && Math.abs(video.currentTime - state!.target) > state!.tolerance) {
        try { video.currentTime = state!.target; } catch { /* Detached media. */ }
      }
    };
    video.addEventListener?.('seeked', () => {
      // A slow decoder may immediately start the coalesced request below.
      // Preserve its completed pixels before readyState drops for that seek.
      if (state!.active && video.paused && !video.seeking && video.readyState >= 2 && video.videoWidth && video.videoHeight) {
        const canvas = state!.frame?.canvas ?? document.createElement('canvas');
        if (canvas.width !== video.videoWidth) canvas.width = video.videoWidth;
        if (canvas.height !== video.videoHeight) canvas.height = video.videoHeight;
        try {
          canvas.getContext('2d')!.drawImage(video, 0, 0);
          state!.frame = { canvas, time: video.currentTime, version: ++state!.version };
        } catch { /* An unavailable decoder will retry on its next completion. */ }
      }
      resume();
    });
    video.addEventListener?.('loadedmetadata', resume);
  }
  state.target = target; state.tolerance = tolerance; state.active = true;
  if (!video.seeking && Math.abs(video.currentTime - target) > tolerance) {
    try { video.currentTime = target; } catch { /* Detached media. */ }
  }
}

/** Export and normal playback take exclusive ownership of decoder time. */
export function cancelPreviewVideoSeek(video: HTMLVideoElement): void {
  const state = pending.get(video);
  if (state) {
    state.active = false;
    if (state.frame) { state.frame.canvas.width = 0; state.frame.canvas.height = 0; state.frame = undefined; }
  }
}

/** The latest completed seek remains drawable while a newer one is decoding. */
export function previewSeekFrame(video: HTMLVideoElement): SeekFrame | undefined {
  const state = pending.get(video);
  return state?.active ? state.frame : undefined;
}
