/*
 * The app's own still captures through the engine ("Use New Engine" on;
 * the Phase 3c render transport, RFC 16).
 *
 * `PM.Export.snapshot` and `PM.Export.snapshotAsync` serve the agent's
 * `look` and `observe` and the Library's section thumbnails. With the new
 * engine on they draw through the engine, never the current compositor.
 *
 * The exporter installs after the engine connection
 * (`legacy/bootstrap.ts`) and assigns its methods after it assigns
 * `PM.Export`, so they cannot be swapped from here. Instead the exporter's
 * own `snapshot` and `snapshotAsync` ask `PM.EngineCaptures` first
 * (`legacy/core/exporter.ts`): whatever wraps or replaces them (an
 * extension override by id) still reaches the engine through them, in any
 * install order.
 */
import type { PMRegistry } from '../legacy/registry';

export type CaptureEngine = {
  snapshotDataUrlSync(time: number, maxWidth: number, size: { w: number; h: number }): string;
  snapshotDataUrl(time: number, maxWidth: number, size: { w: number; h: number }): Promise<string>;
};

/** `PM.EngineCaptures`: what the exporter's captures draw with. */
export type EngineCaptures = {
  /** Today's `snapshot(T, maxW)`: a JPEG data URL at quality 0.74. */
  snapshot(time: number, maxWidth?: number): string;
  snapshotAsync(time: number, maxWidth?: number): Promise<string>;
};

/** Installs `PM.EngineCaptures`; returns its removal. */
export function installAppCaptures(PM: PMRegistry, engine: CaptureEngine): () => void {
  const captures: EngineCaptures = {
    snapshot: (time, maxWidth = 480) => engine.snapshotDataUrlSync(Number(time) || 0, Number(maxWidth) || 480, PM.proj),
    snapshotAsync: (time, maxWidth = 480) => engine.snapshotDataUrl(Number(time) || 0, Number(maxWidth) || 480, PM.proj)
  };
  PM.EngineCaptures = captures;
  return () => {
    if (PM.EngineCaptures === captures) PM.EngineCaptures = null;
  };
}
