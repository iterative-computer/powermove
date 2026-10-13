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
 * Imports are released with `release()`, which also tells main (Electron's
 * own notice), so `allReferencesReleased` fires once main released its
 * reference too: when the viewer retires a generation, when a surface of
 * a newer engine launch arrives (the old engine's pools are gone), and when
 * the page goes away (`pagehide`). Electron sends the release notice from
 * a callback that does not run once the page unloads, so main cannot rely
 * on it for a reloading page and reclaims those surfaces itself (see
 * main/engine/bridge.ts); releasing here still frees the renderer's
 * references when the notice does get through.
 */
import { isFrameSurfaceArgs, type FrameBridgeApi, type FrameBridgeStats } from '../shared/frame-bridge';

/** Electron's `SharedTextureImported`, as the receiver uses it. */
export interface ImportedLike {
  getVideoFrame(): VideoFrame;
  release(): void;
}

/** Electron's renderer `sharedTexture` module. */
export interface ReceiverModule {
  setSharedTextureReceiver(callback: (data: { importedSharedTexture: ImportedLike }, ...args: unknown[]) => Promise<void>): void;
}

const MAX_ERRORS = 8;

/** Where the receiver hears that its page is going away. */
export type PageLifecycle = { addEventListener(type: 'pagehide', listener: () => void): void };

export function installFrameBridgeReceiver(module: ReceiverModule, page: PageLifecycle | null = typeof window === 'undefined' ? null : window): FrameBridgeApi {
  let launch = 0;
  const imports = new Map<string, ImportedLike>();
  const listeners = new Set<(generation: number, poolIndex: number) => void>();
  const stats: FrameBridgeStats = { launch: 0, held: 0, received: 0, released: 0, videoFrames: 0, misses: 0, errors: [] };
  const key = (generation: number, poolIndex: number): string => `${generation}:${poolIndex}`;
  const fail = (message: string): void => {
    if (stats.errors.length < MAX_ERRORS) stats.errors.push(message);
  };
  const release = (k: string): void => {
    const imported = imports.get(k);
    if (!imported) return;
    imports.delete(k);
    stats.released++;
    try {
      imported.release();
    } catch (error) {
      fail(`release: ${error instanceof Error ? error.message : String(error)}`);
    }
  };

  // A reload or navigation: hand every surface back, or it stays allocated.
  page?.addEventListener('pagehide', () => {
    for (const k of [...imports.keys()]) release(k);
  });

  module.setSharedTextureReceiver(async ({ importedSharedTexture }, ...args) => {
    const info = args[0];
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
    }
    const k = key(info.generation, info.poolIndex);
    if (imports.has(k)) release(k);
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
