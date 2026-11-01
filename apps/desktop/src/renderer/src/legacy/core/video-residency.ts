import { cancelPreviewVideoSeek } from './video-seek';
import { stopPlaybackVideoFrames } from './video-playback-frames';

/*
 * Decoder residency.
 *
 * Every HTMLVideoElement that has shown a frame keeps a hardware decoder, its
 * output surfaces and demuxer buffers alive until its source is removed. A
 * cut-heavy edit touches dozens of clips, and nothing ever let those go: one
 * 6-minute talking-head edit held more than 100 live decoders (about 1.5 GB
 * across the GPU and editor processes) after a little scrubbing.
 *
 * Elements are "parked" once they have been idle for a while and more than a
 * few are resident: parking removes the source, which frees the decoder while
 * keeping the element (and every reference to it) valid. The next use wakes it
 * by restoring the same source and position. Callers already treat a video
 * that is still loading as "not ready yet", so a wake costs one short reload
 * instead of a code path of its own.
 */

type Parked = { src: string; preload: HTMLVideoElement['preload']; time: number };

const parked = new WeakMap<HTMLVideoElement, Parked>();
const tracked = new Map<HTMLVideoElement, number>(); // element -> last use (ms)

/** Elements used within this window are never parked. */
export const RESIDENT_IDLE_MS = 4000;
/** Recently used decoders kept warm beyond the idle window, newest first. */
export const RESIDENT_WARM = 8;
/** Even the warm set is released after this long without use. */
export const RESIDENT_MAX_IDLE_MS = 45000;
/** A fast scrub across many clips may not hold more decoders than this. */
export const RESIDENT_CAP = 12;
/** ...except ones used this recently, which are certainly still on screen. */
const CAP_GRACE_MS = 500;

const parkListeners = new Set<(el: HTMLVideoElement) => void>();
/** Lets owners of per-element resources (GPU textures) release them too. */
export function onVideoParked(listener: (el: HTMLVideoElement) => void): () => void {
  parkListeners.add(listener);
  return () => parkListeners.delete(listener);
}

const now = () => (typeof performance !== 'undefined' ? performance.now() : Date.now());
// The src attribute is authoritative: currentSrc can outlive an unload.
const liveSource = (el: HTMLVideoElement): string =>
  (typeof el.getAttribute === 'function' ? el.getAttribute('src') : el.src) || '';

/** Start tracking an element that holds (or will hold) a decoder. */
export function trackVideo(el: HTMLVideoElement | null | undefined, at = now()): void {
  if (!el) return;
  if (!tracked.has(el)) tracked.set(el, at);
}

/** Stop tracking an element whose owner disposed it. */
export function forgetVideo(el: HTMLVideoElement | null | undefined): void {
  if (!el) return;
  tracked.delete(el);
  parked.delete(el);
}

export function isVideoParked(el: HTMLVideoElement | null | undefined): boolean {
  return !!el && parked.has(el);
}

/** The element's source, including while it is parked. */
export function videoSource(el: HTMLVideoElement): string {
  return parked.get(el)?.src || liveSource(el);
}

/** Mark an element as in use, restoring its decoder if it was parked. */
export function useVideo<T extends HTMLVideoElement | null | undefined>(el: T, at = now()): T {
  if (!el) return el;
  tracked.set(el, at);
  const state = parked.get(el);
  if (state) {
    parked.delete(el);
    el.preload = state.preload || 'auto';
    el.src = state.src;
    // Before metadata arrives this sets the default start position, so the
    // element resumes where it was rather than at zero.
    if (state.time > 0) { try { el.currentTime = state.time; } catch { /* reloading */ } }
  }
  return el;
}

/** Restore a released decoder and wait until its metadata is known. */
export function wakeVideo(el: HTMLVideoElement, timeoutMs = 5000): Promise<void> {
  const wasParked = parked.has(el);
  useVideo(el);
  if (!wasParked || el.readyState >= 1) return Promise.resolve();
  return new Promise(resolve => {
    const done = () => { clearTimeout(timer); el.removeEventListener('loadedmetadata', done); el.removeEventListener('error', done); resolve(); };
    const timer = setTimeout(done, timeoutMs);
    el.addEventListener('loadedmetadata', done);
    el.addEventListener('error', done);
  });
}

function park(el: HTMLVideoElement): boolean {
  const src = liveSource(el);
  if (!src || parked.has(el)) return false;
  parked.set(el, { src, preload: el.preload, time: Number(el.currentTime) || 0 });
  cancelPreviewVideoSeek(el);
  stopPlaybackVideoFrames(el);
  try { el.pause(); } catch { /* detached */ }
  el.removeAttribute('src');
  try { el.load(); } catch { /* detached */ }
  for (const listener of parkListeners) { try { listener(el); } catch { /* best effort */ } }
  return true;
}

/**
 * Release decoders nobody is using. `keep` protects elements the caller is
 * about to use. Returns how many decoders were released.
 */
export function parkIdleVideos(keep: ReadonlySet<HTMLVideoElement> = new Set(), at = now(), options: {
  idleMs?: number; warm?: number; maxIdleMs?: number; cap?: number;
} = {}): number {
  const idleMs = options.idleMs ?? RESIDENT_IDLE_MS;
  const warm = options.warm ?? RESIDENT_WARM;
  const maxIdleMs = options.maxIdleMs ?? RESIDENT_MAX_IDLE_MS;
  const cap = options.cap ?? RESIDENT_CAP;
  const resident: Array<[HTMLVideoElement, number]> = [];
  for (const [el, used] of tracked) {
    if (parked.has(el)) continue;
    // Disposed by its owner (source removed outside this module).
    if (!liveSource(el)) { tracked.delete(el); continue; }
    resident.push([el, used]);
  }
  resident.sort((a, b) => b[1] - a[1]);
  let released = 0, warmKept = 0, remaining = resident.length;
  for (const [el, used] of resident) {
    // Playing or seeking elements belong to someone right now.
    if (keep.has(el) || !el.paused || el.seeking) continue;
    const idle = at - used;
    if (idle < idleMs) continue;
    if (warmKept < warm && idle < maxIdleMs) { warmKept++; continue; }
    if (park(el)) { released++; remaining--; }
  }
  // Least recently used first, release past the cap.
  for (let index = resident.length - 1; index >= 0 && remaining > cap; index--) {
    const [el, used] = resident[index]!;
    if (parked.has(el) || keep.has(el) || !el.paused || el.seeking || at - used < CAP_GRACE_MS) continue;
    if (park(el)) { released++; remaining--; }
  }
  return released;
}

/** Rough decoder cost: a handful of decoded surfaces plus demuxer buffers. */
function decoderBytes(el: HTMLVideoElement): number {
  const pixels = (el.videoWidth || 1920) * (el.videoHeight || 1080);
  return pixels * 1.5 * 12 + 4 * 1024 * 1024;
}

/** For the memory registry, diagnostics and tests. */
export function videoResidency(): { tracked: number; parked: number; resident: number; bytes: number } {
  let parkedCount = 0, bytes = 0;
  for (const el of tracked.keys()) {
    if (parked.has(el)) parkedCount++;
    else if (liveSource(el)) bytes += decoderBytes(el);
  }
  return { tracked: tracked.size, parked: parkedCount, resident: tracked.size - parkedCount, bytes };
}

/** Under memory pressure only decoders used in the last moment survive. */
export function releaseVideoDecoders(targetBytes: number, keep: ReadonlySet<HTMLVideoElement> = new Set()): number {
  if (videoResidency().bytes <= targetBytes) return 0;
  const warm = Math.max(0, Math.floor(targetBytes / decoderBytes({ videoWidth: 1920, videoHeight: 1080 } as HTMLVideoElement)));
  return parkIdleVideos(keep, now(), { idleMs: 500, warm, maxIdleMs: Infinity });
}
