import { validSequenceFps } from './image-sequence';

/* Animated GIF, APNG, animated WebP and animated AVIF all give every frame its
   own delay, while the editor's proxy encoder speaks a single frame rate. These
   helpers pick a rate that reproduces the original timing and say how many
   sequence slots each decoded frame has to occupy to get there. */

/** GIF's "no delay" convention, resolved the way browsers render it. */
export const DEFAULT_FRAME_MS = 100;
const MIN_FRAME_MS = 10;
/** Frames the proxy encoder will hold still for, so a 1 ms delay cannot
    explode a short animation into tens of thousands of encoded frames. */
export const MAX_SEQUENCE_FRAMES = 2400;
const MIN_RESAMPLE_FPS = 12;
const MAX_RESAMPLE_FPS = 120;

export interface AnimationTiming {
  fps: number;
  /** Sequence slots each decoded frame occupies, in frame order. */
  repeats: number[];
  /** Total slots, i.e. the frame count of the encoded proxy. */
  frames: number;
}

function greatestCommonDivisor(a: number, b: number): number {
  while (b) { const remainder = a % b; a = b; b = remainder; }
  return a;
}

/** Decoders report microseconds, and occasionally report nothing at all. */
export function frameDelaysMs(durations: Array<number | null | undefined>): number[] {
  return durations.map(value => {
    const ms = Math.round(Number(value ?? 0) / 1000);
    return Number.isFinite(ms) && ms >= MIN_FRAME_MS ? ms : DEFAULT_FRAME_MS;
  });
}

export function animationTiming(
  durations: Array<number | null | undefined>,
  maxFrames = MAX_SEQUENCE_FRAMES,
): AnimationTiming {
  const delays = frameDelaysMs(durations);
  if (!delays.length) throw new Error('This animation has no frames');
  /* The common case is one shared delay, or a handful of delays that share a
     divisor. That rate reproduces every frame boundary exactly. */
  const unit = delays.reduce(greatestCommonDivisor);
  const fps = 1000 / unit;
  const repeats = delays.map(ms => ms / unit);
  const frames = repeats.reduce((total, count) => total + count, 0);
  if (validSequenceFps(fps) && frames <= maxFrames) return { fps, repeats, frames };
  return resampled(delays, maxFrames);
}

/** Oversample instead: frame starts land within half a slot of the original. */
function resampled(delays: number[], maxFrames: number): AnimationTiming {
  const seconds = delays.reduce((total, ms) => total + ms, 0) / 1000;
  const average = delays.length / Math.max(seconds, 1 / MAX_RESAMPLE_FPS);
  let fps = Math.min(MAX_RESAMPLE_FPS, Math.max(MIN_RESAMPLE_FPS, Math.round(average * 4)));
  if (fps * seconds > maxFrames) fps = Math.max(1, Math.floor(maxFrames / Math.max(seconds, 1 / MAX_RESAMPLE_FPS)));
  const repeats: number[] = [];
  let elapsed = 0, placed = 0;
  for (const ms of delays) {
    elapsed += ms;
    // Round against elapsed time, never per frame, so error cannot accumulate.
    const slot = Math.max(1, Math.round((elapsed / 1000) * fps) - placed);
    repeats.push(slot);
    placed += slot;
  }
  return { fps, repeats, frames: placed };
}
