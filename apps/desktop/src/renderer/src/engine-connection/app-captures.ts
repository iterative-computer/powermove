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
 * resumes it), and an aborted `signal` rejects it as the current capture
 * does (the engine's draw finishes on its own; the capture stays busy until
 * then). Blender rendering is retired (main 2b1fa379a): a
 * composition's `render3d` settings are inert and 3D draws natively in
 * both captures.
 */
import type { PMRegistry } from '../legacy/registry';

export type CaptureEngine = {
  snapshotDataUrlSync(time: number, maxWidth: number, size: { w: number; h: number }, options?: { captionsHidden?: boolean }): string;
  snapshotDataUrl(time: number, maxWidth: number, size: { w: number; h: number }, options?: { prepareBlender?: boolean; captionsHidden?: boolean }): Promise<string>;
};

/** Today's refusal while an export or another frame capture runs. */
export const CAPTURE_BUSY = 'Wait for the current frame capture or export to finish.';

/** `PM.EngineCaptures`: what the exporter's captures draw with. */
export type EngineCaptures = {
  /** Today's `snapshot(T, maxW)`: a JPEG data URL at quality 0.74. */
  snapshot(time: number, maxWidth?: number): string;
  snapshotAsync(time: number, maxWidth?: number, options?: { signal?: AbortSignal }): Promise<string>;
};

const cancelled = (signal: AbortSignal): unknown => signal.reason ?? new DOMException('Frame capture cancelled', 'AbortError');

/** `promise`, or the signal's reason once it aborts. */
function abortable<T>(promise: Promise<T>, signal: AbortSignal): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const abort = () => reject(cancelled(signal));
    signal.addEventListener('abort', abort, { once: true });
    promise.then(resolve, reject).finally(() => signal.removeEventListener('abort', abort));
  });
}

/** Installs `PM.EngineCaptures`; returns its removal. */
export function installAppCaptures(PM: PMRegistry, engine: CaptureEngine, afterSnapshot?: (time: number) => void): () => void {
  const captures: EngineCaptures = {
    snapshot: (time, maxWidth = 480) => {
      const at = Number(time) || 0;
      const image = engine.snapshotDataUrlSync(at, Number(maxWidth) || 480, PM.proj, { captionsHidden: PM.captionsHidden === true });
      afterSnapshot?.(at);
      return image;
    },
    snapshotAsync: async (time, maxWidth = 480, options = {}) => {
      const { signal } = options;
      if (signal?.aborted) throw cancelled(signal);
      if (PM.Export?.busy || PM.agentFrameCapture) throw new Error(CAPTURE_BUSY);
      const wasPlaying = !!PM.playing;
      PM.pause?.();
      PM.agentFrameCapture = true;
      const captionsHidden = PM.captionsHidden === true;
      const release = () => {
        PM.agentFrameCapture = false;
        if (wasPlaying) PM.play?.();
      };
      let capture: Promise<string>;
      try {
        capture = engine.snapshotDataUrl(Number(time) || 0, Number(maxWidth) || 480, PM.proj, { captionsHidden });
      } catch (error) {
        release();
        throw error;
      }
      const settled = capture.then(image => { afterSnapshot?.(Number(time) || 0); return image; }).finally(release);
      return signal ? abortable(settled, signal) : settled;
    }
  };
  PM.EngineCaptures = captures;
  return () => {
    if (PM.EngineCaptures === captures) PM.EngineCaptures = null;
  };
}
