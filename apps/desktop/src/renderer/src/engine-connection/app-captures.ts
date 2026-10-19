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
 *
 * `snapshotAsync` keeps today's contract: it refuses while an export or
 * another frame capture runs, pauses playback while it captures (and
 * resumes it), and a Blender composition's frame is rendered by Blender
 * first (today's `prepareFrame`; the engine's Blender stage, the same one
 * behind the paused viewer's Rendered shading), then captured. `snapshot`
 * draws a frame an earlier capture prepared, or fails with today's error.
 */
import { engineBlenderCapture } from '../core/scene3d/rendering';
import type { PMRegistry } from '../legacy/registry';

export type CaptureEngine = {
  snapshotDataUrlSync(time: number, maxWidth: number, size: { w: number; h: number }, options?: { captionsHidden?: boolean }): string;
  snapshotDataUrl(time: number, maxWidth: number, size: { w: number; h: number }, options?: { prepareBlender?: boolean; captionsHidden?: boolean }): Promise<string>;
};

/** Today's refusal while an export or another frame capture runs. */
export const CAPTURE_BUSY = 'Wait for the current frame capture or export to finish.';

/** Whether `project` has a composition Blender renders (`render3d.enabled`):
 *  the open one or any in `comps`. */
export function usesBlender(project: { render3d?: { enabled?: unknown } | null; comps?: Record<string, { render3d?: { enabled?: unknown } | null } | null> | null } | null | undefined): boolean {
  if (!project) return false;
  if (project.render3d?.enabled) return true;
  return Object.values(project.comps ?? {}).some((comp) => !!comp?.render3d?.enabled);
}

/** `PM.EngineCaptures`: what the exporter's captures draw with. */
export type EngineCaptures = {
  /** Today's `snapshot(T, maxW)`: a JPEG data URL at quality 0.74. */
  snapshot(time: number, maxWidth?: number): string;
  snapshotAsync(time: number, maxWidth?: number): Promise<string>;
};

/** Installs `PM.EngineCaptures`; returns its removal. */
export function installAppCaptures(PM: PMRegistry, engine: CaptureEngine): () => void {
  const captures: EngineCaptures = {
    snapshot: (time, maxWidth = 480) => engine.snapshotDataUrlSync(Number(time) || 0, Number(maxWidth) || 480, PM.proj, { captionsHidden: PM.captionsHidden === true }),
    snapshotAsync: async (time, maxWidth = 480) => {
      if (PM.Export?.busy || PM.agentFrameCapture) throw new Error(CAPTURE_BUSY);
      const wasPlaying = !!PM.playing;
      PM.pause?.();
      PM.agentFrameCapture = true;
      const captionsHidden = PM.captionsHidden === true;
      try {
        const capture = () => engine.snapshotDataUrl(Number(time) || 0, Number(maxWidth) || 480, PM.proj, { prepareBlender: true, captionsHidden });
        return await (usesBlender(PM.proj) ? engineBlenderCapture(PM, capture) : capture());
      } finally {
        PM.agentFrameCapture = false;
        if (wasPlaying) PM.play?.();
      }
    }
  };
  PM.EngineCaptures = captures;
  return () => {
    if (PM.EngineCaptures === captures) PM.EngineCaptures = null;
  };
}
