type SeekFrame = { canvas: HTMLCanvasElement; time: number; version: number };
type SeekState = { target: number; tolerance: number; active: boolean; frames: SeekFrame[]; version: number };
const pending = new WeakMap<HTMLVideoElement, SeekState>();

function matchingFrame(state: SeekState | undefined, target: number, tolerance: number): SeekFrame | undefined {
  return state?.active ? state.frames.find(frame => Math.abs(frame.time - target) <= tolerance) : undefined;
}

/** Keep the newest scrub request without repeatedly flushing an in-flight decode. */
export function seekPreviewVideo(video: HTMLVideoElement, target: number, tolerance: number): void {
  let state = pending.get(video);
  if (!state) {
    state = { target, tolerance, active: true, frames: [], version: 0 };
    pending.set(video, state);
    const resume = () => {
      if (state!.active && video.paused && !video.seeking
          && !matchingFrame(state, state!.target, state!.tolerance)
          && Math.abs(video.currentTime - state!.target) > state!.tolerance) {
        try { video.currentTime = state!.target; } catch { /* Detached media. */ }
      }
    };
    video.addEventListener?.('seeked', () => {
      // A slow decoder may immediately start the coalesced request below.
      // Preserve its completed pixels before readyState drops for that seek.
      if (state!.active && video.paused && !video.seeking && video.readyState >= 2 && video.videoWidth && video.videoHeight) {
        const existing = matchingFrame(state, video.currentTime, state!.tolerance);
        // A second frame lets a quick reverse scrub reuse decoded pixels while
        // the decoder finishes the newer request. Full-size 4K sources keep one.
        const capacity = video.videoWidth * video.videoHeight <= 1280 * 1280 ? 2 : 1;
        const recycled = existing || (state!.frames.length >= capacity ? state!.frames.at(-1) : undefined);
        const canvas = recycled?.canvas ?? document.createElement('canvas');
        if (canvas.width !== video.videoWidth) canvas.width = video.videoWidth;
        if (canvas.height !== video.videoHeight) canvas.height = video.videoHeight;
        try {
          const context = canvas.getContext('2d')!;
          // Recycled frames must replace transparent pixels too. Source-over
          // leaves earlier silhouettes behind and accumulates partial alpha.
          context.globalCompositeOperation = 'copy';
          context.drawImage(video, 0, 0);
          if (recycled) state!.frames.splice(state!.frames.indexOf(recycled), 1);
          state!.frames.unshift({ canvas, time: video.currentTime, version: ++state!.version });
        } catch { /* An unavailable decoder will retry on its next completion. */ }
      }
      resume();
    });
    video.addEventListener?.('loadedmetadata', resume);
  }
  state.target = target; state.tolerance = tolerance; state.active = true;
  if (!video.seeking && !matchingFrame(state, target, tolerance)
      && Math.abs(video.currentTime - target) > tolerance) {
    try { video.currentTime = target; } catch { /* Detached media. */ }
  }
}

/** Export and normal playback take exclusive ownership of decoder time. */
export function cancelPreviewVideoSeek(video: HTMLVideoElement): void {
  const state = pending.get(video);
  if (state) {
    state.active = false;
    for (const frame of state.frames) { frame.canvas.width = 0; frame.canvas.height = 0; }
    state.frames.length = 0;
  }
}

/** Only pixels from the requested source time may stand in for a busy decoder. */
export function previewSeekFrame(video: HTMLVideoElement, target?: number, tolerance = .0005): SeekFrame | undefined {
  const state = pending.get(video);
  return target === undefined ? (state?.active ? state.frames[0] : undefined)
    : matchingFrame(state, target, tolerance);
}
