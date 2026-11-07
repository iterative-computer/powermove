import type { PowermoveAPI } from 'powermove';
import type { MaterialMapValue } from './materials';

/*
 * Small previews of material sources for the picker, the closed slot buttons
 * and the Timeline's Materials rows. The engine renders them lazily, one per
 * idle turn, and answers null while it is busy (playback, a drag, export) or
 * while a source is still loading. This keeps the last good picture on screen,
 * asks again after a project edit, and retries a null a few times with a
 * growing pause, so a row shows its placeholder until the picture is ready and
 * never flickers back to it.
 */

/** Logical size requested for every surface, so they share one cached capture. */
export const THUMBNAIL_SIZE = 32;
const MAX_TRIES = 5;
const MAX_ENTRIES = 160;

interface Entry {
  url: string | null;
  /** Project revision the current request (or picture) belongs to. */
  stamp: number;
  pending: boolean;
  tries: number;
  retry: ReturnType<typeof setTimeout> | null;
}

export interface MaterialThumbnails {
  /** The picture for a source, or null (show the placeholder). Asks the engine when it is missing or stale. */
  peek(receiverId: string, map: MaterialMapValue | null | undefined): string | null;
  /** Called whenever a picture arrives. */
  subscribe(listener: () => void): () => void;
  dispose(): void;
}

export const thumbnailSourceKey = (map: MaterialMapValue): string =>
  typeof map === 'string' ? `image:${map}` : 'compositionId' in map ? `comp:${map.compositionId}` : `layer:${map.layerId}`;

export function createMaterialThumbnails(api: PowermoveAPI): MaterialThumbnails {
  const entries = new Map<string, Entry>();
  const listeners = new Set<() => void>();
  let disposed = false;
  const revision = (): number => { try { return Number(api.project.revision?.()) || 0; } catch { return 0; } };
  const notify = (): void => { for (const listener of [...listeners]) listener(); };

  function trim(): void {
    while (entries.size > MAX_ENTRIES) {
      const [oldest, entry] = entries.entries().next().value as [string, Entry];
      if (entry.retry) clearTimeout(entry.retry);
      entries.delete(oldest);
    }
  }

  function request(key: string, entry: Entry, receiverId: string, map: MaterialMapValue): void {
    entry.pending = true;
    const stamp = entry.stamp;
    let promise: Promise<{ url: string } | null>;
    try {
      promise = api.scene3d.getMaterialThumbnail(receiverId, map as any, { size: THUMBNAIL_SIZE, pixelRatio: Math.min(2, window.devicePixelRatio || 1) }) as Promise<{ url: string } | null>;
    } catch {
      promise = Promise.resolve(null);
    }
    void promise.catch(() => null).then((result) => {
      if (disposed || entries.get(key) !== entry) return;
      entry.pending = false;
      if (result?.url) {
        const changed = entry.url !== result.url;
        entry.url = result.url; entry.tries = 0;
        if (changed) notify();
      }
      // The project moved on while this was rendering: ask for the current picture.
      if (entry.stamp !== stamp) { request(key, entry, receiverId, map); return; }
      if (result?.url) return;
      // Busy or still loading: keep what is shown and ask again shortly.
      if (entry.tries >= MAX_TRIES) return;
      entry.tries++;
      entry.retry = setTimeout(() => {
        entry.retry = null;
        if (disposed || entries.get(key) !== entry || entry.pending) return;
        request(key, entry, receiverId, map);
      }, 350 * 2 ** (entry.tries - 1));
    });
  }

  return {
    peek(receiverId, map) {
      if (disposed || map == null) return null;
      const key = `${receiverId}|${thumbnailSourceKey(map)}`;
      const now = revision();
      let entry = entries.get(key);
      if (!entry) {
        entry = { url: null, stamp: now, pending: false, tries: 0, retry: null };
        entries.set(key, entry); trim();
        request(key, entry, receiverId, map);
      } else if (entry.stamp !== now) {
        // The project changed: refresh, keeping the old picture until the new one lands.
        entry.stamp = now; entry.tries = 0;
        if (entry.retry) { clearTimeout(entry.retry); entry.retry = null; }
        if (!entry.pending) request(key, entry, receiverId, map);
      } else {
        // Most recently used last, so trimming drops the oldest.
        entries.delete(key); entries.set(key, entry);
      }
      return entry.url;
    },
    subscribe(listener) {
      listeners.add(listener);
      return () => { listeners.delete(listener); };
    },
    dispose() {
      disposed = true;
      for (const entry of entries.values()) if (entry.retry) clearTimeout(entry.retry);
      entries.clear(); listeners.clear();
    }
  };
}

let shared: { api: PowermoveAPI; thumbnails: MaterialThumbnails } | null = null;
/** One cache per extension activation, shared by the inspector, the picker and the timeline service. */
export function materialThumbnails(api: PowermoveAPI): MaterialThumbnails {
  if (shared?.api !== api) { shared?.thumbnails.dispose(); shared = { api, thumbnails: createMaterialThumbnails(api) }; }
  return shared.thumbnails;
}
export function disposeMaterialThumbnails(): void { shared?.thumbnails.dispose(); shared = null; }
