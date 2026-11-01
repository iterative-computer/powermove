import { Blake3 } from '../../../shared/blake3';
import { Sha256 } from '../../../shared/sha256';
import type { TransitionBridge, TRPayload } from '../../../shared/transition';
const CHUNK = 1024 * 1024;
const keyHash = (text: string) => new Sha256().update(new TextEncoder().encode(text)).digest();
export async function blobHash(blob: Blob): Promise<string> {
  const hash = new Blake3();
  for (let at = 0; at < blob.size; at += CHUNK) {
    hash.update(new Uint8Array(await blob.slice(at, at + CHUNK).arrayBuffer()));
    await new Promise(resolve => setTimeout(resolve, 0));
  }
  return hash.hex();
}
export async function transfer(host: TransitionBridge, payload: TRPayload, blob: Blob, pass?: string, knownHash?: string): Promise<string> {
  const hash = knownHash ?? await blobHash(blob);
  const reused = pass && await host.request({ action: 'reuse', pass, payload, hash });
  if (reused) return reused;
  const token = await host.request({ action: 'begin', payload, pass });
  try {
    for (let at = 0; at < blob.size; at += CHUNK) await host.request({ action: 'chunk', token, offset: at,
      data: new Uint8Array(await blob.slice(at, at + CHUNK).arrayBuffer()) });
    return await host.request({ action: 'finish', token, hash });
  } finally { await host.request({ action: 'abort', token }).catch(() => undefined); }
}
export function exportLocalStorage(storage: Storage): Record<string, { raw: string; json?: unknown }> {
  const values: Record<string, { raw: string; json?: unknown }> = Object.create(null);
  for (let index = 0; index < storage.length; index++) {
    const key = storage.key(index);
    if (key === null) throw new Error('Browser settings changed during export.');
    const raw = storage.getItem(key);
    if (raw === null) throw new Error('Browser settings changed during export.');
    values[key] = { raw };
    try { values[key]!.json = JSON.parse(raw); } catch { /* exact DOM string is authoritative */ }
  }
  return values;
}
type Entry = { locator: string; key: string; value: any };
export async function browserRecords(factory: IDBFactory): Promise<Entry[]> {
  const db = await new Promise<IDBDatabase>((resolve, reject) => {
    const request = factory.open('powermove-media');
    let settled = false;
    const fail = (error: unknown) => { if (!settled) { settled = true; clearTimeout(timer); reject(error); } };
    const timer = setTimeout(() => fail(new Error('Older media is busy. Preparation will resume.')), 5000);
    request.onsuccess = () => {
      if (settled) { request.result.close(); return; }
      settled = true; clearTimeout(timer); resolve(request.result);
    };
    request.onerror = () => fail(request.error);
    request.onblocked = () => fail(new Error('Close other Powermove windows to prepare older media.'));
  });
  try {
    const names = Array.from(db.objectStoreNames);
    if (!names.length) return [];
    return await new Promise((resolve, reject) => {
      const tx = db.transaction(names, 'readonly');
      const entries: Entry[] = [];
      for (const name of names) {
        const request = tx.objectStore(name).openCursor();
        request.onsuccess = () => {
          const cursor = request.result;
          if (!cursor) return;
          try {
            const key = typeof cursor.primaryKey === 'string' ? cursor.primaryKey : JSON.stringify(logical(cursor.primaryKey).metadata);
            entries.push({ locator: JSON.stringify([location.origin, name, logical(cursor.primaryKey).metadata]), key,
              value: cursor.value });
            cursor.continue();
          } catch (error) { reject(error); tx.abort(); }
        };
      }
      tx.oncomplete = () => resolve(entries.sort((a, b) => a.locator.localeCompare(b.locator)));
      tx.onabort = tx.onerror = () => reject(tx.error ?? new Error('Older media could not be read.'));
    });
  } finally { db.close(); }
}
/** Structured-clone values become inspectable logical JSON; Blob bytes stay streamed. */
export function logical(value: unknown): { metadata: unknown; blobs: { blob: Blob; path: string }[] } {
  const seen = new Map<object, number>(), blobs: { blob: Blob; path: string }[] = [];
  const visit = (v: any, path: string): any => {
    if (v === undefined) return { $type: 'undefined' };
    if (typeof v === 'bigint') return { $type: 'bigint', value: String(v) };
    if (typeof v === 'number' && (!Number.isFinite(v) || Object.is(v, -0))) return { $type: 'number', value: Object.is(v, -0) ? '-0' : String(v) };
    if (!v || typeof v !== 'object') return v;
    if (seen.has(v)) return { $ref: seen.get(v) };
    const id = seen.size; seen.set(v, id);
    if (v instanceof Blob) {
      blobs.push({ blob: v, path });
      return { $id: id, $type: 'blob', path, size: v.size, type: v.type,
        ...(typeof File !== 'undefined' && v instanceof File ? { name: v.name, lastModified: v.lastModified } : {}) };
    }
    if (v instanceof Date) return { $id: id, $type: 'date', value: Number.isNaN(v.getTime()) ? 'NaN' : v.toISOString() };
    if (v instanceof ArrayBuffer || ArrayBuffer.isView(v)) return { $id: id, $type: v.constructor.name,
      value: Array.from(v instanceof ArrayBuffer ? new Uint8Array(v) : new Uint8Array(v.buffer, v.byteOffset, v.byteLength)) };
    if (v instanceof Map) return { $id: id, $type: 'map', value: [...v].map(([k, x], i) => [visit(k, `${path}/key${i}`), visit(x, `${path}/value${i}`)]) };
    if (v instanceof Set) return { $id: id, $type: 'set', value: [...v].map((x, i) => visit(x, `${path}/${i}`)) };
    if (Array.isArray(v)) {
      const properties = Object.keys(v).filter(k => !/^(0|[1-9][0-9]*)$/.test(k) || Number(k) >= v.length);
      return { $id: id, $type: 'array', value: Array.from({ length: v.length }, (_, i) => i in v ? visit(v[i], `${path}/${i}`) : { $type: 'hole' }),
        ...(properties.length ? { properties: Object.fromEntries(properties.sort().map(k => [k, visit((v as any)[k], `${path}/${k.replaceAll('~', '~0').replaceAll('/', '~1')}`)])) } : {}) };
    }
    if (Object.getPrototypeOf(v) !== Object.prototype && Object.getPrototypeOf(v) !== null) throw new Error('An older media record requires a newer logical exporter. Its data has been preserved.');
    return { $id: id, $type: 'object', value: Object.fromEntries(Object.keys(v).sort().map(k => [k, visit(v[k], `${path}/${k.replaceAll('~', '~0').replaceAll('/', '~1')}`)])) };
  };
  return { metadata: visit(value, ''), blobs };
}
export async function prepareTransition(host: TransitionBridge, records: () => Promise<Entry[]>, storage: Storage,
  progress: (done: number, total: number) => void): Promise<void> {
  const entries = await records();
  const inventory = entries.map(e => e.locator);
  const pass = await host.request({ action: 'start', inventory });
  const signatures = new Map<string, string>();
  let done = 0;
  for (const entry of entries) {
    const { metadata, blobs } = logical(entry.value);
    const payloads: string[] = [], hashes: string[] = [];
    for (const { blob, path } of blobs) {
      const hash = await blobHash(blob); hashes.push(hash);
      const key = path === '/blob' && /^[A-Za-z0-9:._-]{1,300}$/.test(entry.key) ? entry.key : `idb:${keyHash(entry.locator + path)}`;
      payloads.push(await transfer(host, { key, size: blob.size, type: blob.type, revision: entry.value?.revision,
        locator: entry.locator, metadata: { record: metadata, path },
        ...(entry.value?.playbackProxy || entry.value?.playbackProxyVersion > 0 ? { proxyOf: {
          fingerprint: entry.value.fingerprint ?? null, sourcePath: entry.value.sourcePath ?? null } } : {}) }, blob, pass, hash));
    }
    signatures.set(entry.locator, keyHash(JSON.stringify([metadata, hashes])));
    await host.request({ action: 'record', pass, locator: entry.locator, metadata, payloads });
    progress(++done, entries.length);
    await new Promise(resolve => setTimeout(resolve, 0));
  }
  // Re-read every record, including unknown stores and nested Blob fields.
  const after = await records();
  for (const entry of after) {
    const { metadata, blobs } = logical(entry.value);
    const hashes = [];
    for (const { blob } of blobs) hashes.push(await blobHash(blob));
    if (signatures.get(entry.locator) !== keyHash(JSON.stringify([metadata, hashes]))) throw new Error('Media changed during preparation. A delta pass will resume automatically.');
  }
  await host.request({ action: 'localStorage', values: exportLocalStorage(storage) });
  await host.request({ action: 'complete', pass, inventory: after.map(e => e.locator) });
}
export function installTransitionNotice(PM: any, host: TransitionBridge): () => void {
  let stopped = false, timer: ReturnType<typeof setTimeout> | undefined;
  const run = async () => {
    try {
      const status = await host.request({ action: 'status' });
      // Stable completed libraries must not be rehashed every minute. New TR
      // imports invalidate this state; native still owns the final origin delta.
      if (status.mediaComplete) {
        await host.request({ action: 'localStorage', values: exportLocalStorage(window.localStorage) });
      } else await prepareTransition(host, () => browserRecords(window.indexedDB), window.localStorage, (done, total) => {
        if (!stopped) PM.toast('Preparing your library', 2500, { key: 'transition', task: true,
          progress: done / Math.max(1, total), detail: `${done} of ${total} media records`, error: false });
      });
      PM.dismissToast?.('transition');
    } catch {
      if (!stopped) PM.toast('Library preparation paused. It will resume automatically.', 6000, { key: 'transition', kind: 'status', error: false });
    }
    // New TR imports invalidate readiness and trigger another delta pass.
    if (!stopped) timer = setTimeout(() => { void run(); }, 60_000);
  };
  void Promise.resolve(PM.projectReady).then(() => { if (!stopped) timer = setTimeout(() => { void run(); }, 5000); });
  return () => { stopped = true; if (timer) clearTimeout(timer); };
}
