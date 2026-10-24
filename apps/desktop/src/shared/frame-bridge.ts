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

/** A one-shot source-video import, distinct from the viewer's reusable pool. */
export type SourceSurfaceArgs = {
  kind: 'powermove-source-surface';
  token: string;
  width: number;
  height: number;
};

export function isSourceSurfaceArgs(value: unknown): value is SourceSurfaceArgs {
  if (typeof value !== 'object' || value === null) return false;
  const v = value as Record<string, unknown>;
  return v['kind'] === 'powermove-source-surface'
    && typeof v['token'] === 'string' && /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(v['token'])
    && typeof v['width'] === 'number' && Number.isInteger(v['width']) && v['width'] > 0 && v['width'] <= 16384
    && typeof v['height'] === 'number' && Number.isInteger(v['height']) && v['height'] > 0 && v['height'] <= 16384;
}

export type FrameBridgeStats = {
  launch: number;
  held: number;
  received: number;
  released: number;
  videoFrames: number;
  misses: number;
  /** Deliveries of an import already held (main retried a send that timed
   *  out but landed); each is let go without a second notice to main. */
  duplicates: number;
  /** Release notices main no longer had an import for (harmless: the
   *  renderer's own reference is gone either way). */
  lateReleases: number;
  errors: string[];
};

export interface FrameBridgeApi {
  /** A `VideoFrame` of a surface of the current launch, or null when its
   *  import has not arrived (or was retired). Close it after drawing. */
  videoFrame(generation: number, poolIndex: number): VideoFrame | null;
  /** Takes one source VideoFrame. Close it, then call discardSource to release
   *  its renderer import after presentation. Each token can be taken once. */
  sourceFrame(token: string): VideoFrame | null;
  /** Releases a consumed source import or one the monitor will not display. */
  discardSource(token: string): void;
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
    count(v['poolSize'], 64) &&
    count(v['poolIndex'], 63) &&
    (v['poolIndex'] as number) < (v['poolSize'] as number) &&
    count(v['width'], 16384) &&
    count(v['height'], 16384) &&
    (v['format'] === 'bgra' || v['format'] === 'rgbaf16')
  );
}
