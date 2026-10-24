/** One-shot source-video imports in the preload's isolated world. */
import { isSourceSurfaceArgs, type SourceSurfaceArgs } from '../shared/frame-bridge';
import type { ImportedLike, NotifyRelease, PageLifecycle } from './frame-bridge';

export const MAX_SOURCE_IMPORTS = 64;
export const SOURCE_IMPORT_TTL_MS = 10_000;

export type SourceTimers = {
  set(callback: () => void, delay: number): unknown;
  clear(handle: unknown): void;
};
const browserTimers: SourceTimers = {
  set: (callback, delay) => setTimeout(callback, delay),
  clear: handle => clearTimeout(handle as ReturnType<typeof setTimeout>)
};

type Held = { imported: ImportedLike; args: SourceSurfaceArgs; timeout: unknown; taken: boolean };

export interface SourceSurfaceReceiver {
  receive(imported: ImportedLike, args: unknown): void;
  sourceFrame(token: string): VideoFrame | null;
  discardSource(token: string): void;
  stats(): { held: number; received: number; released: number; expired: number; errors: string[] };
}

export function installSourceSurfaceReceiver(
  page: PageLifecycle | null,
  notify: NotifyRelease | null,
  timers: SourceTimers = browserTimers
): SourceSurfaceReceiver {
  const held = new Map<string, Held>();
  const stats = { received: 0, released: 0, expired: 0, errors: [] as string[] };
  const fail = (error: unknown) => {
    if (stats.errors.length < 8) stats.errors.push(error instanceof Error ? error.message : String(error));
  };
  const letGo = (imported: ImportedLike, notice = true) => {
    try {
      if (imported.subtle && typeof imported.textureId === 'string' && (!notice || notify)) {
        // Electron's release callback can run when the final VideoFrame is
        // destroyed on VideoFrameCompositor, outside this JS sequence. Never
        // pass a JS callback into that path. The imported VideoFrame retains
        // the shared image, while Electron retained the immutable IOSurface
        // at import; main's notice only retires its bookkeeping reference.
        imported.subtle.release();
        if (notice && notify) {
          const id = imported.textureId;
          void Promise.resolve().then(() => notify(id)).catch(fail);
        }
      } else imported.release();
    } catch (error) { fail(error); }
  };
  const drop = (token: string) => {
    const entry = held.get(token);
    if (!entry) return;
    held.delete(token);
    timers.clear(entry.timeout);
    stats.released++;
    letGo(entry.imported);
  };
  page?.addEventListener('pagehide', () => { for (const token of [...held.keys()]) drop(token); });
  return {
    receive(imported, args) {
      if (!isSourceSurfaceArgs(args)) { fail('Invalid source surface descriptor'); letGo(imported); return; }
      stats.received++;
      const previous = held.get(args.token);
      if (previous) {
        // A transfer that timed out in main can still have arrived. Main only
        // counted one renderer reference for the same Electron texture id.
        if (previous.imported.textureId && previous.imported.textureId === imported.textureId) {
          letGo(imported, false);
          return;
        }
        drop(args.token);
      }
      while (held.size >= MAX_SOURCE_IMPORTS) drop(held.keys().next().value!);
      const timeout = timers.set(() => { stats.expired++; drop(args.token); }, SOURCE_IMPORT_TTL_MS);
      held.set(args.token, { imported, args, timeout, taken: false });
    },
    sourceFrame(token) {
      const entry = held.get(token);
      if (!entry || entry.taken) return null;
      entry.taken = true;
      try {
        const frame = entry.imported.getVideoFrame();
        if (!frame) drop(token);
        return frame;
      } catch (error) {
        fail(error);
        drop(token);
        return null;
      }
    },
    discardSource: drop,
    stats: () => ({ ...stats, held: held.size, errors: [...stats.errors] })
  };
}
