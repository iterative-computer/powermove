// @vitest-environment happy-dom
import { createHash } from 'node:crypto';

import { describe, expect, it, vi } from 'vitest';

import { blake3Hex } from '../../../shared/blake3';
import type { MediaLink, MediaStoreBridge, MediaStoreRecord } from '../../../shared/media-store';
import { bindingsFor, createHostMediaStore, decideImport, hashBlob, renditionKey, type LegacyMediaStore } from './host-media-store';

/** An in-memory stand-in for main's store with the same contract. */
function fakeHost(options: { links?: Record<string, Uint8Array>; moved?: Record<string, string> } = {}) {
  const objects = new Map<string, Uint8Array>();
  const keys = new Map<string, MediaStoreRecord>();
  const uploads = new Map<string, { key: string | null; type: string; size: number; parts: Uint8Array[]; revision: string | null; origin: string }>();
  const reads = new Map<string, Uint8Array>();
  const files = new Map<string, Uint8Array>(Object.entries(options.links ?? {}));
  const projectFiles = new Map<string, Uint8Array>();
  let next = 0;
  const calls: string[] = [];
  const commit = (key: string | null, data: Uint8Array, type: string, revision: string | null, source: string): MediaStoreRecord => {
    const hash = blake3Hex(data);
    objects.set(hash, data);
    const record = { key: key ?? `blake3:${hash}`, hash, size: data.length, type, revision: revision || hash, at: 1, source } as MediaStoreRecord;
    if (key) keys.set(key, record);
    return record;
  };
  const host: MediaStoreBridge = {
    configSync: () => ({ enabled: true }),
    record: async (key) => keys.get(key) ?? null,
    classify: async (path) => (path.includes('/Agent Workspaces/') ? { storage: 'copied', reason: 'agentWorkspace' } : { storage: 'linked' }),
    putFile: async ({ path, key, type }) => { calls.push(`putFile:${path}`); return commit(key, files.get(path)!, type ?? '', null, 'import'); },
    beginPut: async ({ key, type, size, revision, origin }) => { const token = String(++next).padStart(32, '0'); uploads.set(token, { key, type, size, parts: [], revision: revision ?? null, origin: origin ?? 'stream' }); calls.push(`beginPut:${key}:${origin}`); return token; },
    putChunk: async (token, offset, data) => { const u = uploads.get(token)!; expect(offset).toBe(u.parts.reduce((s, p) => s + p.length, 0)); u.parts.push(data.slice()); },
    finishPut: async (token, expected) => {
      const u = uploads.get(token)!;
      uploads.delete(token);
      const data = new Uint8Array(u.parts.reduce((s, p) => s + p.length, 0));
      let at = 0; for (const p of u.parts) { data.set(p, at); at += p.length; }
      const sha256 = createHash('sha256').update(data).digest('hex');
      if (expected.blake3 !== undefined ? blake3Hex(data) !== expected.blake3 : expected.sha256 !== undefined && expected.sha256 !== sha256) {
        throw new Error(expected.blake3 ? 'The stored copy does not match the original bytes.' : 'The project media checksum does not match.');
      }
      return { ...commit(u.key, data, u.type, u.revision, u.origin), sha256 };
    },
    putProjectRange: async (token, readToken, offset, length, at) => {
      calls.push(`putProjectRange:${readToken}:${offset}:${length}:${at}`);
      const u = uploads.get(token)!;
      const file = projectFiles.get(readToken)!;
      u.parts.push(file.slice(offset, offset + length));
    },
    abortPut: async (token) => { uploads.delete(token); },
    link: async (path) => ({ path, hash: blake3Hex(files.get(path)!), size: files.get(path)!.length, mtimeMs: 1 }),
    open: async (request) => {
      let data: Uint8Array | undefined; let hash = ''; let type = ''; let relinked: MediaLink | undefined;
      if (request.link) {
        const moved = options.moved?.[request.link.path];
        data = files.get(moved ?? request.link.path);
        if (data && blake3Hex(data) !== request.link.hash) return { ok: false, status: 'changed' };
        if (!data) return { ok: false, status: 'offline' };
        hash = request.link.hash;
        if (moved) relinked = { ...request.link, path: moved };
      } else if (request.key) {
        const record = keys.get(request.key);
        if (!record) return { ok: false, status: 'missing' };
        data = objects.get(record.hash); hash = record.hash; type = record.type;
      } else if (request.hash) {
        data = objects.get(request.hash); hash = request.hash;
        if (!data) return { ok: false, status: 'missing' };
      }
      const token = String(++next).padStart(32, '0');
      reads.set(token, data!);
      return { ok: true, token, size: data!.length, type, hash, ...(relinked ? { relinked } : {}) };
    },
    read: async (token, offset, length) => reads.get(token)!.slice(offset, offset + length),
    close: async (token) => { reads.delete(token); },
    relink: async () => null,
    bind: async () => undefined,
    removeKey: async (key) => keys.delete(key)
  };
  return { host, objects, keys, files, calls, projectFiles };
}

function fakeLegacy(initial: Record<string, Blob> = {}) {
  const records = new Map<string, { blob: Blob; revision: string }>(Object.entries(initial).map(([k, blob]) => [k, { blob, revision: `idb-${k}` }]));
  const puts: string[] = [];
  const removes: string[] = [];
  const key = (value: any) => (value && typeof value === 'object' ? value.storageKey || value.id : value);
  const legacy: LegacyMediaStore = {
    put: async (id, blob, meta = {}) => { puts.push(meta.storageKey || id); records.set(meta.storageKey || id, { blob, revision: 'r' }); return true; },
    get: async (value) => records.get(key(value))?.blob ?? null,
    getForSave: async (value) => records.get(key(value)) ?? null,
    remove: async (value) => { removes.push(key(value)); return records.delete(key(value)); }
  };
  return { legacy, records, puts, removes };
}

const bytesOf = async (blob: Blob | null) => (blob ? new Uint8Array(await blob.arrayBuffer()) : null);
const data = (length: number, seed = 1) => Uint8Array.from({ length }, (_, i) => (i * 31 + seed) & 255);

describe('host media store (new engine on)', () => {
  it('stores puts in the host store, never in IndexedDB, verified by hash', async () => {
    const { host, keys } = fakeHost();
    const { legacy, puts } = fakeLegacy();
    const store = createHostMediaStore({ host, legacy, enabled: true, sourcePath: () => null });
    const payload = data(9 * 1024 * 1024 + 11);
    expect(await store.put('a1', new Blob([payload], { type: 'video/mp4' }), { storageKey: 'media:v2:k' })).toBe(true);
    expect(puts).toEqual([]);
    expect(keys.get('media:v2:k')).toMatchObject({ hash: blake3Hex(payload), size: payload.length, type: 'video/mp4' });
    expect(store.recordFor('media:v2:k')?.hash).toBe(blake3Hex(payload));
    const back = await bytesOf(await store.get({ id: 'a1', storageKey: 'media:v2:k' }));
    expect(back && back.length === payload.length && back.every((byte, i) => byte === payload[i])).toBe(true);
  });

  it('lets main copy a file that has a path instead of sending its bytes', async () => {
    const payload = data(1000);
    const { host, calls } = fakeHost({ links: { '/Media/clip.mov': payload } });
    const { legacy } = fakeLegacy();
    const store = createHostMediaStore({ host, legacy, enabled: true, sourcePath: () => '/Media/clip.mov' });
    const file = new File([payload], 'clip.mov', { type: 'video/quicktime' });
    expect(await store.put('a1', file, { storageKey: 'media:v2:c' })).toBe(true);
    expect(calls).toEqual(['putFile:/Media/clip.mov']);
  });

  it('copies IndexedDB media into the store, verified, and never deletes or writes IndexedDB', async () => {
    const old = new Blob([data(5000, 3)], { type: 'image/png' });
    const { host, keys, calls } = fakeHost();
    const { legacy, records, puts, removes } = fakeLegacy({ 'media:v2:old': old });
    const store = createHostMediaStore({ host, legacy, enabled: true, sourcePath: () => null });
    // A read falls back to IndexedDB and copies in the background.
    expect(await bytesOf(await store.get({ id: 'a1', storageKey: 'media:v2:old' }))).toEqual(data(5000, 3));
    await vi.waitFor(() => expect(keys.get('media:v2:old')).toBeTruthy());
    expect(keys.get('media:v2:old')).toMatchObject({ hash: blake3Hex(data(5000, 3)), revision: 'idb-media:v2:old', source: 'indexeddb' });
    expect(calls.filter((c) => c.startsWith('beginPut'))).toEqual(['beginPut:media:v2:old:indexeddb']);
    expect(records.has('media:v2:old')).toBe(true);
    expect(puts).toEqual([]);
    expect(removes).toEqual([]);
    // Explicit migration counts what was there.
    const counts = await store.migrate([{ id: 'a1', storageKey: 'media:v2:old' }, { id: 'a2', storageKey: 'media:v2:none' }, { id: 'a3', storage: 'linked', link: { path: '/x', hash: 'a'.repeat(64), size: 1 } }]);
    expect(counts).toEqual({ copied: 0, present: 1, missing: 1, failed: 0 });
  });

  it('reads linked originals, reports relinks, and falls back to the same content in the store', async () => {
    const original = data(3000, 9);
    const hash = blake3Hex(original);
    const { host } = fakeHost({ links: { '/Media/new place.mov': original }, moved: { '/Media/A001.mov': '/Media/new place.mov' } });
    const { legacy } = fakeLegacy();
    const relinked = vi.fn();
    const store = createHostMediaStore({ host, legacy, enabled: true, sourcePath: () => null, onRelinked: relinked });
    const meta = { id: 'a1', storageKey: 'media:v2:l', storage: 'linked', link: { path: '/Media/A001.mov', hash, size: original.length } };
    expect(await bytesOf(await store.get(meta))).toEqual(original);
    expect(relinked).toHaveBeenCalledWith(meta, expect.objectContaining({ path: '/Media/new place.mov', hash }));
    // Desktop Save stores no bytes for a linked asset.
    expect(await store.getForSave(meta)).toBeNull();
    expect(await store.saveSource(meta)).toBeNull();
    // Offline original, same content embedded in the store: still readable.
    const offline = { ...meta, link: { ...meta.link, path: '/Gone/A001.mov' } };
    expect(await store.get(offline)).toBeNull();
    await store.put('a1', new Blob([original]), { storageKey: 'other-key' });
    expect(await bytesOf(await store.get(offline))).toEqual(original);
  });

  it('keeps browser renditions as generated media under their own key', async () => {
    const { host, keys } = fakeHost();
    const store = createHostMediaStore({ host, legacy: fakeLegacy().legacy, enabled: true, sourcePath: () => null });
    expect(await store.rendition.get('media:v2:p')).toBeNull();
    expect(await store.rendition.put('media:v2:p', new Blob([data(10)], { type: 'video/webm' }))).toBe(true);
    expect(keys.get(renditionKey('media:v2:p'))).toMatchObject({ source: 'rendition', type: 'video/webm' });
    expect(await bytesOf(await store.rendition.get('media:v2:p'))).toEqual(data(10));
  });

  it('names stored bytes for desktop Save, migrating IndexedDB media first', async () => {
    const { host } = fakeHost();
    const store = createHostMediaStore({ host, legacy: fakeLegacy({ 'media:v2:s': new Blob([data(40)]) }).legacy, enabled: true, sourcePath: () => null });
    expect(await store.saveSource({ id: 'a', storageKey: 'media:v2:s' })).toMatchObject({ hash: blake3Hex(data(40)), size: 40, revision: 'idb-media:v2:s' });
  });
});

describe('opening a project file', () => {
  it('has main copy embedded ranges into the store, checked against the recorded checksum', async () => {
    const { host, keys, calls, projectFiles } = fakeHost();
    const store = createHostMediaStore({ host, legacy: fakeLegacy().legacy, enabled: true, sourcePath: () => null });
    const media = data(300, 4);
    const file = new Uint8Array(1000); file.set(media, 200);
    projectFiles.set('reader-1', file);
    const sha256 = createHash('sha256').update(media).digest('hex');
    const progress: number[] = [];
    const source = { id: 'a1', type: 'audio/wav', offset: 200, length: 300, revision: 'rev-7', sha256 };
    expect(await store.putProjectRange('reader-1', source, { id: 'a1', storageKey: 'media:v2:w' }, (bytes) => progress.push(bytes))).toBe(true);
    expect(calls).toContain('putProjectRange:reader-1:200:300:0');
    expect(keys.get('media:v2:w')).toMatchObject({ hash: blake3Hex(media), revision: 'rev-7', source: 'project' });
    expect(progress).toEqual([300]);
    // A range that is not what the file recorded is refused, and nothing is kept.
    await expect(store.putProjectRange('reader-1', { ...source, offset: 201 }, { id: 'a1', storageKey: 'media:v2:x' })).rejects.toThrow(/checksum/);
    expect(keys.has('media:v2:x')).toBe(false);
  });

  it('leaves ranges to the page with the flag off or without host support', async () => {
    const { host } = fakeHost();
    const off = createHostMediaStore({ host, legacy: fakeLegacy().legacy, enabled: false, sourcePath: () => null });
    expect(await off.putProjectRange('reader-1', { id: 'a', type: '', offset: 0, length: 1 }, { id: 'a' })).toBe(false);
    const { putProjectRange: _omit, ...older } = host;
    const on = createHostMediaStore({ host: older as MediaStoreBridge, legacy: fakeLegacy().legacy, enabled: true, sourcePath: () => null });
    expect(await on.putProjectRange('reader-1', { id: 'a', type: '', offset: 0, length: 1 }, { id: 'a' })).toBe(false);
  });
});

describe('host media store (new engine off)', () => {
  it('keeps IndexedDB as the source and reads the store only for its own media', async () => {
    const { host, keys } = fakeHost();
    const { legacy, puts } = fakeLegacy({ 'media:v2:old': new Blob([data(10, 1)]) });
    // Media imported with the flag on.
    const on = createHostMediaStore({ host, legacy, enabled: true, sourcePath: () => null });
    await on.put('a2', new Blob([data(20, 2)]), { storageKey: 'media:v2:new' });
    const off = createHostMediaStore({ host, legacy, enabled: false, sourcePath: () => null });
    expect(await bytesOf(await off.get({ id: 'a1', storageKey: 'media:v2:old' }))).toEqual(data(10, 1));
    expect(await bytesOf(await off.get({ id: 'a2', storageKey: 'media:v2:new' }))).toEqual(data(20, 2));
    expect((await off.getForSave({ id: 'a2', storageKey: 'media:v2:new' }))?.revision).toBe(keys.get('media:v2:new')!.revision);
    await off.put('a3', new Blob([data(5)]), { storageKey: 'media:v2:legacy' });
    expect(puts).toEqual(['media:v2:legacy']);
    expect(keys.has('media:v2:legacy')).toBe(false);
  });
});

describe('import decisions', () => {
  const link = (path: string) => ({ links: { [path]: data(64) } });
  it('copies by default and links only when asked, never the always-copied cases', async () => {
    const { host } = fakeHost(link('/Media/a.mov'));
    const base = { enabled: true, kind: 'video', sourcePath: '/Media/a.mov' };
    expect(await decideImport(host, base)).toEqual({ storage: 'copied', reason: 'setting' });
    expect(await decideImport(host, { ...base, projectSetting: 'link' })).toMatchObject({ storage: 'linked', link: { path: '/Media/a.mov', hash: blake3Hex(data(64)) } });
    expect(await decideImport(host, { ...base, projectSetting: 'link', requested: 'copy' })).toEqual({ storage: 'copied', reason: 'requested' });
    expect(await decideImport(host, { ...base, requested: 'link' })).toMatchObject({ storage: 'linked' });
    expect(await decideImport(host, { ...base, projectSetting: 'link', sourcePath: null })).toEqual({ storage: 'copied', reason: 'notAFile' });
    expect(await decideImport(host, { ...base, projectSetting: 'link', kind: 'model' })).toEqual({ storage: 'copied', reason: 'model' });
    expect(await decideImport(host, { ...base, projectSetting: 'link', converted: true })).toEqual({ storage: 'copied', reason: 'conversion' });
    expect(await decideImport(host, { ...base, projectSetting: 'link', sourcePath: '/u/Agent Workspaces/p/a.mov' })).toEqual({ storage: 'copied', reason: 'agentWorkspace' });
    expect(await decideImport(host, { ...base, enabled: false, requested: 'link' })).toEqual({ storage: 'copied' });
  });

  it('binds copied assets by key and hash and linked assets by link', () => {
    const link = { path: '/Media/a.mov', hash: 'b'.repeat(64), size: 3 };
    const bindings = bindingsFor({ assets: {
      a1: { id: 'a1', storageKey: 'media:v2:1', contentHash: 'a'.repeat(64) },
      a2: { id: 'a2', storage: 'linked', link },
      a3: { id: 'a3', storageKey: 'media:v2:3' }
    } }, (key) => (key === 'media:v2:3' ? { key, hash: 'c'.repeat(64), size: 1, type: '', revision: 'r', at: 0 } : null));
    expect(bindings).toEqual({ a1: { key: 'media:v2:1', hash: 'a'.repeat(64) }, a2: { link }, a3: { key: 'media:v2:3', hash: 'c'.repeat(64) } });
  });

  it('hashes a blob in slices to the same digest', async () => {
    const payload = data(5 * 1024 * 1024 + 3);
    const seen: number[] = [];
    expect(await hashBlob(new Blob([payload]), async (chunk, offset) => { seen.push(offset); })).toBe(blake3Hex(payload));
    expect(seen).toEqual([0, 4 * 1024 * 1024]);
  });
});
