/*
 * The frame bridge's renderer side (plan section 18, amendment 4), as the
 * preload exposes it on the `powermove` bridge (`frameBridge`).
 *
 * Main imports each engine pool surface once and sends it once to the
 * window whose viewer it serves; the preload keeps every import for the
 * pool's lifetime. Per frame, the viewer asks for a `VideoFrame` of the
 * surface its descriptor names. The kernel captures the bridge at boot, so
 * extensions reach this only through the kernel's API.
 */

/** What the preload receives with each import (main's `sendSharedTexture` args). */
export type FrameSurfaceArgs = {
  kind: 'powermove-frame-surface';
  /** The interactive engine launch (pool generations restart with it). */
  launch: number;
  generation: number;
  poolIndex: number;
  poolSize: number;
  width: number;
  height: number;
  format: 'bgra' | 'rgbaf16';
};

export type FrameBridgeStats = {
  launch: number;
  held: number;
  received: number;
  released: number;
  videoFrames: number;
  misses: number;
  errors: string[];
};

export interface FrameBridgeApi {
  /** A `VideoFrame` of a surface of the current launch, or null when its
   *  import has not arrived (or was retired). Close it after drawing. */
  videoFrame(generation: number, poolIndex: number): VideoFrame | null;
  has(generation: number, poolIndex: number): boolean;
  /** Called with (generation, poolIndex) whenever an import arrives. */
  onSurface(listener: (generation: number, poolIndex: number) => void): () => void;
  /** Releases the imports of a generation the engine replaced. */
  retire(generation: number): number;
  stats(): FrameBridgeStats;
}

export function isFrameSurfaceArgs(value: unknown): value is FrameSurfaceArgs {
  if (typeof value !== 'object' || value === null) return false;
  const v = value as Record<string, unknown>;
  const count = (x: unknown, max: number): boolean => typeof x === 'number' && Number.isInteger(x) && x >= 0 && x <= max;
  return (
    v['kind'] === 'powermove-frame-surface' &&
    count(v['launch'], 2 ** 31) &&
    count(v['generation'], 2 ** 32) &&
    count(v['poolSize'], 32) &&
    count(v['poolIndex'], 31) &&
    (v['poolIndex'] as number) < (v['poolSize'] as number) &&
    count(v['width'], 16384) &&
    count(v['height'], 16384) &&
    (v['format'] === 'bgra' || v['format'] === 'rgbaf16')
  );
}
