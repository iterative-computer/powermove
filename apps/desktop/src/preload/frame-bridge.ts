/*
 * The frame bridge's receiver (plan section 18, item 5; amendment 4). Runs
 * in the preload's isolated world, where `setSharedTextureReceiver` works
 * in a sandboxed, context-isolated renderer (S1).
 *
 * Each surface main sends arrives once and stays imported for the pool's
 * lifetime: the viewer makes a `VideoFrame` from it per descriptor
 * (`getVideoFrame()` sees the surface's current contents; the engine never
 * writes a surface the viewer holds). `VideoFrame`s cross to the main world
 * through the context bridge natively.
 *
 * Imports are released with a notice to main (Electron's own channel), so
 * `allReferencesReleased` fires once main released its reference too: when
 * the viewer retires a generation, when a surface of a newer engine launch
 * arrives (the old engine's pools are gone), and when the page goes away
 * (`pagehide`).
 *
 * Main counts one renderer reference per transfer that completed in time.
 * A transfer that timed out on main's side (1000 ms; this page's main
 * thread was busy, or the GPU process was restarting) can still land here,
 * and main then sends the import again: the same texture arrives twice
 * while main counted it once. The receiver keeps the first and lets go of
 * the copy without a notice, so every counted reference gets exactly one
 * (phase2-gate.md D3: the extra notice reached main after it had dropped
 * the texture). Any notice main can no longer place is counted as a late
 * release rather than left as an unhandled rejection: the renderer's
 * reference is released either way. Electron sends the release notice from
 * a callback that does not run once the page unloads, so main cannot rely
 * on it for a reloading page and reclaims those surfaces itself (see
 * main/engine/bridge.ts); releasing here still frees the renderer's
 * references when the notice does get through.
 */
import { isFrameSurfaceArgs, type FrameBridgeApi, type FrameBridgeStats } from '../shared/frame-bridge';
import { installSourceSurfaceReceiver, type SourceSurfaceReceiver } from './source-surfaces';

/** Electron's `SharedTextureImported`, as the receiver uses it. */
export interface ImportedLike {
  getVideoFrame(): VideoFrame;
  /** Electron's release: the renderer's reference, then its notice to main. */
  release(): void;
  /** The texture's id in main (Electron 44's renderer wrapper). */
  readonly textureId?: string;
  /** The renderer's reference alone. Source video imports release without a
   *  callback; a VideoFrame retains its own reference until consumption ends. */
  readonly subtle?: { release(callback?: () => unknown): void };
}

/** Tells main that this renderer let go of the texture `id` (Electron's own
 *  channel, `IMPORT_SHARED_TEXTURE_RELEASE_RENDERER_TO_MAIN`). */
export type NotifyRelease = (textureId: string) => Promise<unknown>;

/** Electron's channel for a renderer's release notice (its renderer
 *  `sharedTexture` wrapper invokes it the same way; S1). */
export const RELEASE_NOTICE_CHANNEL = 'IMPORT_SHARED_TEXTURE_RELEASE_RENDERER_TO_MAIN';

/** Electron's renderer `sharedTexture` module. */
export interface ReceiverModule {
  setSharedTextureReceiver(callback: (data: { importedSharedTexture: ImportedLike }, ...args: unknown[]) => Promise<void>): void;
}

const MAX_ERRORS = 8;

/** Where the receiver hears that its page is going away. */
export type PageLifecycle = { addEventListener(type: 'pagehide', listener: () => void): void };

/**
 * `notify`: sends the release notice; without it (or for an import that
 * lacks Electron's renderer handle) `release()` sends it, unhandled.
 */
export function installFrameBridgeReceiver(
  module: ReceiverModule,
  page: PageLifecycle | null = typeof window === 'undefined' ? null : window,
  notify: NotifyRelease | null = null,
  sourceReceiver: SourceSurfaceReceiver = installSourceSurfaceReceiver(page, notify)
): FrameBridgeApi {
  let launch = 0;
  const imports = new Map<string, ImportedLike>();
  const retired = new Set<number>();
  const listeners = new Set<(generation: number, poolIndex: number) => void>();
  const stats: FrameBridgeStats = { launch: 0, held: 0, received: 0, released: 0, videoFrames: 0, misses: 0, duplicates: 0, lateReleases: 0, errors: [] };
  const key = (generation: number, poolIndex: number): string => `${generation}:${poolIndex}`;
  const fail = (message: string): void => {
    if (stats.errors.length < MAX_ERRORS) stats.errors.push(message);
  };
  /** Lets go of one import: its renderer reference, and (`notice`) main's count of it. */
  const letGo = (imported: ImportedLike, notice: boolean): void => {
    const subtle = imported.subtle;
    const id = imported.textureId;
    try {
      if (subtle && typeof subtle.release === 'function' && typeof id === 'string' && (notify || !notice)) {
        subtle.release(() => {
          if (!notice || !notify) return;
          // Main may have let go of the texture already (its count reached
          // zero through an earlier notice): nothing is left to release.
          return notify(id).catch((error: unknown) => {
            stats.lateReleases++;
            if (!/not found|not referenced/i.test(error instanceof Error ? error.message : String(error))) fail(`release notice: ${error instanceof Error ? error.message : String(error)}`);
          });
        });
      } else {
        imported.release();
      }
    } catch (error) {
      fail(`release: ${error instanceof Error ? error.message : String(error)}`);
    }
  };
  const release = (k: string): void => {
    const imported = imports.get(k);
    if (!imported) return;
    imports.delete(k);
    stats.released++;
    letGo(imported, true);
  };

  // A reload or navigation: hand every surface back, or it stays allocated.
  page?.addEventListener('pagehide', () => {
    for (const k of [...imports.keys()]) release(k);
  });

  module.setSharedTextureReceiver(async ({ importedSharedTexture }, ...args) => {
    const info = args[0];
    if (typeof info === 'object' && info !== null && (info as { kind?: unknown }).kind === 'powermove-source-surface') {
      sourceReceiver.receive(importedSharedTexture, info);
      return;
    }
    if (!isFrameSurfaceArgs(info)) {
      // Not ours (or malformed): hold nothing.
      importedSharedTexture.release();
      fail('a shared texture arrived without frame surface arguments');
      return;
    }
    stats.received++;
    if (info.launch < launch) {
      importedSharedTexture.release();
      return;
    }
    if (info.launch > launch) {
      // A restarted engine: every surface of the old one is gone.
      for (const k of [...imports.keys()]) release(k);
      launch = info.launch;
      stats.launch = launch;
      retired.clear();
    }
    if (retired.has(info.generation)) {
      letGo(importedSharedTexture, true);
      return;
    }
    const k = key(info.generation, info.poolIndex);
    const held = imports.get(k);
    if (held && typeof held.textureId === 'string' && held.textureId === importedSharedTexture.textureId) {
      // The same texture again: main resent it after a transfer that timed
      // out on its side but landed here, and counted it once.
      stats.duplicates++;
      letGo(importedSharedTexture, false);
      return;
    }
    if (held) release(k);
    imports.set(k, importedSharedTexture);
    for (const listener of listeners) {
      try {
        listener(info.generation, info.poolIndex);
      } catch (error) {
        fail(`listener: ${error instanceof Error ? error.message : String(error)}`);
      }
    }
  });

  return {
    sourceFrame: token => sourceReceiver.sourceFrame(token),
    discardSource: token => sourceReceiver.discardSource(token),
    videoFrame(generation, poolIndex) {
      const imported = imports.get(key(generation, poolIndex));
      if (!imported) {
        stats.misses++;
        return null;
      }
      try {
        const frame = imported.getVideoFrame();
        stats.videoFrames++;
        return frame;
      } catch (error) {
        fail(`getVideoFrame: ${error instanceof Error ? error.message : String(error)}`);
        return null;
      }
    },
    has: (generation, poolIndex) => imports.has(key(generation, poolIndex)),
    onSurface(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    retire(generation) {
      retired.add(generation);
      let n = 0;
      for (const k of [...imports.keys()]) {
        if (k.startsWith(`${generation}:`)) {
          release(k);
          n++;
        }
      }
      return n;
    },
    stats: () => ({ ...stats, held: imports.size, errors: [...stats.errors] })
  };
}
