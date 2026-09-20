/** Retain decoded frames, rather than seeking every visible layer on every tick.
 * VideoFrame keeps the pixels and their decoder timestamp together. */
export type PlaybackVideoFrame = { video: HTMLVideoElement; source: VideoFrame; time: number; duration: number; version: number };
type State = { frames: PlaybackVideoFrame[]; callback: number; seek: () => void; capture: () => void };
const states = new WeakMap<HTMLVideoElement, State>();
const MAX_BYTES = 32 * 1024 * 1024;
let version = 0;

export function clearPlaybackVideoFrames(video: HTMLVideoElement): void {
  const state = states.get(video);
  if (!state) return;
  for (const frame of state.frames) frame.source.close();
  state.frames.length = 0;
}

export function stopPlaybackVideoFrames(video: HTMLVideoElement): void {
  const state = states.get(video);
  if (!state) return;
  video.cancelVideoFrameCallback?.(state.callback);
  video.removeEventListener('seeking', state.seek);
  clearPlaybackVideoFrames(video); states.delete(video);
}

export function startPlaybackVideoFrames(video: HTMLVideoElement, fps: number, ready: () => void): boolean {
  if (typeof video.requestVideoFrameCallback !== 'function' || typeof VideoFrame !== 'function') return false;
  if (states.has(video)) return true;
  const state: State = { frames: [], callback: 0, seek: () => clearPlaybackVideoFrames(video), capture: () => {} };
  states.set(video, state);
  video.addEventListener('seeking', state.seek);
  state.capture = () => {
    if (states.get(video) !== state) return;
    if (video.seeking || video.readyState < 2 || !video.videoWidth || !video.videoHeight) return;
    let source: VideoFrame;
    try { source = new VideoFrame(video); } catch { return; }
    const time = source.timestamp / 1e6;
    if (state.frames.some(frame => Math.abs(frame.time - time) < .00001)) { source.close(); return; }
    // At least two frames are needed to reconcile a decoder arriving one frame
    // late. Bound storage by decoded size, independent of clip duration.
    const capacity = Math.max(2, Math.min(8, Math.floor(MAX_BYTES / (source.displayWidth * source.displayHeight * 4))));
    while (state.frames.length >= capacity) state.frames.shift()!.source.close();
    const duration = source.duration != null && source.duration > 0 ? source.duration / 1e6 : 1 / Math.max(1, fps);
    state.frames.push({ video, source, time, duration, version: ++version });
    ready();
  };
  const presented: VideoFrameRequestCallback = () => {
    if (states.get(video) !== state) return;
    state.callback = video.requestVideoFrameCallback(presented);
    state.capture();
  };
  state.callback = video.requestVideoFrameCallback(presented);
  return true;
}

/** Detached media callbacks may be throttled. Sampling the actual VideoFrame
 * on the editor's frame tick also collects frames between those callbacks. */
export function capturePlaybackVideoFrame(video: HTMLVideoElement): void {
  states.get(video)?.capture();
}

/** Missing callbacks are missing frames, not a longer duration for old pixels. */
export function playbackVideoFrameAt(video: HTMLVideoElement, target: number): PlaybackVideoFrame | undefined {
  const state = states.get(video);
  if (!state || video.seeking) return undefined;
  for (let index = state.frames.length - 1; index >= 0; index--) {
    const frame = state.frames[index]!;
    const end = Math.min(frame.time + frame.duration, state.frames[index + 1]?.time ?? Infinity);
    if (frame.time <= target + .0005 && target < end - .0005) return frame;
  }
  return undefined;
}
