type SeekState = { target: number; tolerance: number; active: boolean };
const pending = new WeakMap<HTMLVideoElement, SeekState>();

/** Keep the newest scrub request without repeatedly flushing an in-flight decode. */
export function seekPreviewVideo(video: HTMLVideoElement, target: number, tolerance: number): void {
  let state = pending.get(video);
  if (!state) {
    state = { target, tolerance, active: true };
    pending.set(video, state);
    const resume = () => {
      if (state!.active && video.paused && !video.seeking && Math.abs(video.currentTime - state!.target) > state!.tolerance) {
        try { video.currentTime = state!.target; } catch { /* Detached media. */ }
      }
    };
    video.addEventListener?.('seeked', resume);
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
  if (state) state.active = false;
}
