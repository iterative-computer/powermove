/*
 * Media on a remote host. The renderer keeps imported bytes in this
 * browser's IndexedDB under a content-addressed key. Another device opening
 * the same project has no such bytes, so every put is mirrored to the host
 * and a miss on get is answered from there.
 */
import { WEB, WEB_UPLOAD_CHUNK_BYTES } from '../../../shared/wire';

export interface MediaLink {
  invoke<T = unknown>(channel: string, ...args: unknown[]): Promise<T>;
}

type MediaStore = {
  put(id: string, blob: Blob, meta?: Record<string, unknown>): Promise<boolean>;
  get(value: unknown): Promise<Blob | null>;
  remove?(value: unknown): Promise<boolean>;
};

const storageKey = (value: unknown): string | null => {
  if (value && typeof value === 'object') {
    const record = value as { storageKey?: unknown; id?: unknown };
    return typeof record.storageKey === 'string' ? record.storageKey : typeof record.id === 'string' ? record.id : null;
  }
  return typeof value === 'string' ? value : null;
};

async function uploadBlob(link: MediaLink, key: string, blob: Blob): Promise<void> {
  const id = await link.invoke<string>(WEB.uploadBegin, { name: 'blob', size: blob.size, kind: 'blob' });
  try {
    for (let offset = 0; offset < blob.size; offset += WEB_UPLOAD_CHUNK_BYTES) {
      const data = new Uint8Array(await blob.slice(offset, offset + WEB_UPLOAD_CHUNK_BYTES).arrayBuffer());
      await link.invoke(WEB.uploadChunk, { id, offset, data });
    }
    const hostPath = await link.invoke<string>(WEB.uploadFinish, id);
    await link.invoke(WEB.mediaCommit, { key, path: hostPath, type: blob.type || 'application/octet-stream' });
  } catch (error) {
    await link.invoke(WEB.uploadAbort, id).catch(() => undefined);
    throw error;
  }
}

/** Wraps PM.MediaStore so puts reach the host and misses are filled from it. */
export interface RemoteMediaStore extends MediaStore {
  /** Sends up anything this browser has under these keys that the host lacks. */
  backfill(keys: string[]): Promise<void>;
  /** True while this key is being fetched from the host. */
  pending(key: unknown): boolean;
}

export function attachRemoteMedia(link: MediaLink, store: MediaStore, onChange: () => void = () => {}): RemoteMediaStore {
  const fetching = new Set<string>();
  const mirrored = new Set<string>();
  const inflight = new Map<string, Promise<void>>();

  const mirror = (key: string, blob: Blob): Promise<void> => {
    if (mirrored.has(key)) return Promise.resolve();
    const existing = inflight.get(key);
    if (existing) return existing;
    const job = link.invoke<string[]>(WEB.mediaHas, [key])
      .then((present) => (present.includes(key) ? undefined : uploadBlob(link, key, blob)))
      .then(() => { mirrored.add(key); })
      .catch((error) => { console.warn('[remote-media] mirror failed', key, error); })
      .finally(() => { inflight.delete(key); });
    inflight.set(key, job);
    return job;
  };

  // Another device may still be uploading what this one is asking for.
  const fetchFromHost = async (key: string, attempts = 4): Promise<Blob | null> => {
    for (let attempt = 0; attempt < attempts; attempt++) {
      const response = await fetch(`/__powermove/media?key=${encodeURIComponent(key)}`, { credentials: 'same-origin' });
      if (response.ok) return response.blob();
      if (response.status !== 404) return null;
      await new Promise((resolve) => setTimeout(resolve, 750 * (attempt + 1)));
    }
    return null;
  };

  /* media imported before the host kept copies, or on a host that lost them */
  const backfill = async (keys: string[]): Promise<void> => {
    const wanted = [...new Set(keys.filter((key) => key && !mirrored.has(key)))];
    if (!wanted.length) return;
    let present: string[] = [];
    try { present = await link.invoke<string[]>(WEB.mediaHas, wanted); } catch { return; }
    for (const key of present) mirrored.add(key);
    for (const key of wanted) {
      if (mirrored.has(key)) continue;
      const blob = await store.get({ storageKey: key, id: key }).catch(() => null);
      if (blob && blob.size > 0) await mirror(key, blob);
    }
  };

  const wrapped: RemoteMediaStore = {
    backfill,
    pending: (value) => { const key = storageKey(value); return !!key && fetching.has(key); },
    async put(id, blob, meta = {}) {
      const stored = await store.put(id, blob, meta);
      const key = storageKey({ storageKey: meta['storageKey'], id });
      if (stored && key && blob.size > 0) void mirror(key, blob);
      return stored;
    },
    async get(value) {
      const local = await store.get(value);
      if (local) return local;
      const key = storageKey(value);
      if (!key) return null;
      let remote: Blob | null = null;
      fetching.add(key); onChange();
      try { remote = await fetchFromHost(key); } catch (error) { console.warn('[remote-media] fetch failed', key, error); }
      finally { fetching.delete(key); onChange(); }
      if (!remote) return null;
      const stored = await store.put(key, remote, value && typeof value === 'object' ? (value as Record<string, unknown>) : { storageKey: key });
      mirrored.add(key);
      return stored ? (await store.get(value)) ?? remote : remote;
    },
    ...(store.remove ? { remove: (value: unknown) => store.remove!(value) } : {})
  };
  return wrapped;
}
