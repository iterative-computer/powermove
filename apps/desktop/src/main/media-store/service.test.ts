import { createHash } from 'node:crypto';
import { mkdir, mkdtemp, readFile, rename, rm, utimes, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import { blake3Hex } from '../../shared/blake3';
import { MEDIA_STORE_IPC } from '../../shared/media-store';
import { HostServicesCore } from '../engine/services';
import { CAMERA_CARD_MARKERS, ImportClassifier } from './classify';
import { InProcessExecutor } from './hasher';
import { PROJECT_RANGE_BYTES, registerMediaStoreIpc } from './ipc';
import { findByContent } from './relink';
import { MediaStoreService } from './service';

const executor = new InProcessExecutor();
let root: string;
let userData: string;
let media: string;

beforeEach(async () => {
  root = await mkdtemp(path.join(os.tmpdir(), 'pm-media-service-'));
  userData = path.join(root, 'userData');
  media = path.join(root, 'Footage');
  await mkdir(userData, { recursive: true });
  await mkdir(media, { recursive: true });
});
afterEach(async () => { await rm(root, { recursive: true, force: true }); });

const bytes = (length: number, seed = 1) => {
  const out = new Uint8Array(length);
  let x = seed;
  for (let i = 0; i < length; i++) { x = (Math.imul(x, 1103515245) + 12345) >>> 0; out[i] = x >>> 24; }
  return out;
};
const settledFile = async (file: string, data: Uint8Array) => {
  await mkdir(path.dirname(file), { recursive: true });
  await writeFile(file, data);
  const t = new Date(Date.now() - 60_000);
  await utimes(file, t, t);
};
const service = (options: Partial<ConstructorParameters<typeof MediaStoreService>[0]> = {}) => new MediaStoreService({
  root: path.join(userData, 'Media Store'), userData, executor, enabled: true,
  classify: { diskInfo: async () => null, mountPoint: async () => '/' },
  relink: { spotlight: null },
  ...options
});

describe('ImportClassifier', () => {
  it('links ordinary files, copies agent workspace and app files, and copies camera media', async () => {
    const classifier = new ImportClassifier({ userData, diskInfo: async () => null, mountPoint: async () => '/' });
    const ordinary = path.join(media, 'A001.mov');
    await settledFile(ordinary, bytes(10));
    expect(await classifier.classify(ordinary)).toEqual({ storage: 'linked' });
    const workspace = path.join(userData, 'Agent Workspaces', 'project-1', 'render.mp4');
    await settledFile(workspace, bytes(10));
    expect(await classifier.classify(workspace)).toEqual({ storage: 'copied', reason: 'agentWorkspace' });
    expect(await classifier.classify(path.join(media, 'missing.mov'))).toEqual({ storage: 'copied', reason: 'notAFile' });
    expect(await classifier.classify('relative.mov')).toEqual({ storage: 'copied', reason: 'notAFile' });

    // A card reader's medium is removable; a camera card layout counts on any volume.
    const card = path.join(root, 'CARD');
    const clip = path.join(card, 'PRIVATE', 'M4ROOT', 'CLIP', 'C0001.MP4');
    await settledFile(clip, bytes(10));
    const onCard = new ImportClassifier({ userData, diskInfo: async () => ({ removableMedia: false, internal: false }), mountPoint: async () => card });
    expect(await onCard.classify(clip)).toEqual({ storage: 'copied', reason: 'cameraMedia' });
    const sd = new ImportClassifier({ userData, diskInfo: async () => ({ removableMedia: true, internal: true }), mountPoint: async () => media });
    expect(await sd.classify(ordinary)).toEqual({ storage: 'copied', reason: 'cameraMedia' });
    const ssd = new ImportClassifier({ userData, diskInfo: async () => ({ removableMedia: false, internal: false }), mountPoint: async () => media });
    expect(await ssd.classify(ordinary)).toEqual({ storage: 'linked' });
    expect(CAMERA_CARD_MARKERS).toContain('DCIM');
  });
});

describe('content-hash relink', () => {
  it('finds a moved and renamed original by its hash, never by its name', async () => {
    const original = path.join(media, 'Day 1', 'A001_C003.mov');
    const data = bytes(50_000);
    await settledFile(original, data);
    const hash = blake3Hex(data);
    const link = { path: original, hash, size: data.length };
    // A same-named, same-sized impostor is never chosen.
    await settledFile(path.join(media, 'Day 2', 'A001_C003.mov'), bytes(50_000, 7));
    const moved = path.join(media, 'Selects', 'hero shot.mov');
    await mkdir(path.dirname(moved), { recursive: true });
    await rename(original, moved);
    const hashOf = async (file: string) => { const { hashFile } = await import('./hasher'); return hashFile(executor, file, { settleMs: 0 }); };
    const found = await findByContent(link, [], { hash: hashOf, spotlight: null });
    expect(found?.link).toMatchObject({ path: moved, hash, size: data.length });
    await rm(moved);
    expect(await findByContent(link, [], { hash: hashOf, spotlight: null })).toBeNull();
  });

  it('searches extra roots and Spotlight candidates, confirming each by hash', async () => {
    const data = bytes(3000, 3);
    const elsewhere = path.join(root, 'Elsewhere', 'deep', 'clip.mov');
    await settledFile(elsewhere, data);
    const link = { path: path.join(root, 'Gone', 'clip.mov'), hash: blake3Hex(data), size: data.length };
    const hashOf = async (file: string) => { const { hashFile } = await import('./hasher'); return hashFile(executor, file, { settleMs: 0 }); };
    expect((await findByContent(link, [path.join(root, 'Elsewhere')], { hash: hashOf, spotlight: null }))?.link.path).toBe(elsewhere);
    const viaSpotlight = await findByContent(link, [], { hash: hashOf, spotlight: async (size) => (size === data.length ? [elsewhere] : []), depth: 0 });
    expect(viaSpotlight?.link.path).toBe(elsewhere);
  });
});

describe('MediaStoreService', () => {
  it('links a file in place and refuses to link always-copied files', async () => {
    const svc = service();
    const file = path.join(media, 'b-roll.mov');
    const data = bytes(20_000);
    await settledFile(file, data);
    const link = await svc.link(file);
    expect(link).toMatchObject({ path: file, hash: blake3Hex(data), size: data.length });
    const workspace = path.join(userData, 'Agent Workspaces', 'p', 'out.mp4');
    await settledFile(workspace, data);
    await expect(svc.link(workspace)).rejects.toThrow(/always copied/);
  });

  it('reads a key, a hash, and a link in bounded chunks, relinking a moved original', async () => {
    const svc = service();
    const file = path.join(media, 'take.wav');
    const data = bytes(5 * 1024 * 1024 + 3);
    await settledFile(file, data);
    const record = await svc.store.putFile(file, { key: 'media:v2:1:a', type: 'audio/wav' });
    const readAll = async (request: Parameters<MediaStoreService['open']>[1]) => {
      const opened = await svc.open(9, request);
      if (!opened.ok) return opened;
      const parts: Uint8Array[] = [];
      for (let at = 0; at < opened.size;) { const chunk = await svc.read(9, opened.token, at, 4 * 1024 * 1024); parts.push(chunk); at += chunk.length; }
      await svc.close(9, opened.token);
      return { ...opened, data: Buffer.concat(parts) };
    };
    const byKey = await readAll({ key: 'media:v2:1:a' });
    expect(byKey.ok && Buffer.compare(byKey.data, data)).toBe(0);
    expect(byKey).toMatchObject({ type: 'audio/wav', hash: record.hash });
    const byHash = await readAll({ hash: record.hash });
    expect(byHash.ok && byHash.data.length).toBe(data.length);
    expect(await svc.open(9, { key: 'media:none' })).toEqual({ ok: false, status: 'missing' });

    const link = await svc.link(file);
    const moved = path.join(media, 'Renamed', 'take 2.wav');
    await mkdir(path.dirname(moved), { recursive: true });
    await rename(file, moved);
    const viaLink = await readAll({ link });
    expect(viaLink).toMatchObject({ ok: true, path: moved, relinked: { path: moved, hash: link.hash } });
    await rm(moved);
    expect(await svc.open(9, { link })).toEqual({ ok: false, status: 'offline' });

    const opened = await svc.open(9, { key: 'media:v2:1:a' });
    if (!opened.ok) throw new Error('expected an open read');
    await expect(svc.read(10, opened.token, 0, 10)).rejects.toThrow(/Unknown/);
    await expect(svc.read(9, opened.token, 0, 8 * 1024 * 1024)).rejects.toThrow(/Invalid/);
    await svc.release(9);
    await expect(svc.read(9, opened.token, 0, 10)).rejects.toThrow(/Unknown/);
  });

  it('resolves engine media reads through bindings and mints capability handles for them', async () => {
    const svc = service();
    const copied = path.join(media, 'copied.png');
    const linkedFile = path.join(media, 'linked.mov');
    await settledFile(copied, bytes(1000, 1));
    await settledFile(linkedFile, bytes(2000, 2));
    const record = await svc.store.putFile(copied, { key: 'media:v2:copied' });
    const link = await svc.link(linkedFile);
    svc.bind(1, 'project-1', { a1: { key: 'media:v2:copied', hash: record.hash }, a2: { link } });
    expect(await svc.resolveAsset('a1')).toEqual({ path: svc.store.objectPath(record.hash), contentHash: Uint8Array.from(Buffer.from(record.hash, 'hex')) });
    expect((await svc.resolveAsset('a2'))?.path).toBe(linkedFile);
    expect(await svc.resolveAsset('a3')).toBeNull();

    // The engine gets the file only as a handle bound to its session and port.
    const host = new HostServicesCore({ cacheDirectory: path.join(root, 'cache'), automaticOutputDirectory: path.join(root, 'out'), physicalMemory: 8 * 1024 ** 3, resolveAsset: (asset) => svc.resolveAsset(asset) });
    const grant = await host.serve('interactive', 5, { type: 'mintHandle', kind: { kind: 'mediaRead', asset: 'a2' }, session: 3 }, 3);
    expect(grant).toMatchObject({ type: 'handle', contentHash: Uint8Array.from(Buffer.from(link.hash, 'hex')) });
    const resolved = await host.serve('interactive', 5, { type: 'resolveHandle', handle: (grant as { handle: string }).handle, session: 3 }, 3);
    expect(resolved).toMatchObject({ type: 'handleResolved', path: linkedFile, access: 'read' });
    await expect(host.serve('interactive', 6, { type: 'resolveHandle', handle: (grant as { handle: string }).handle, session: 3 }, 3)).rejects.toThrow(/another port/);
    await expect(host.serve('interactive', 5, { type: 'mintHandle', kind: { kind: 'mediaRead', asset: 'a3' }, session: 3 }, 3)).rejects.toThrow(/not available/);

    // The same asset id bound to different content in two windows is refused.
    svc.bind(2, 'project-2', { a1: { hash: blake3Hex(bytes(5)) } });
    expect(await svc.resolveAsset('a1')).toBeNull();
    expect(await svc.resolveForProject('project-1', 'a1', 1)).not.toBeNull();
    expect(await svc.resolveForProject('missing-project', 'a1', 1)).toBeNull();
    await svc.release(2);
    expect(await svc.resolveAsset('a1')).not.toBeNull();
  });

  it('keeps snapshot media after a project switch and window close, scoped to its job session', async () => {
    const svc = service();
    const file = path.join(media, 'queued.png');
    await settledFile(file, bytes(1000, 1));
    const record = await svc.store.putFile(file, { key: 'media:v2:queued' });
    const link = await svc.link(file);
    const project = { id: 'queued-project', assets: {
      copied: { id: 'copied', storageKey: record.key }, linked: { id: 'linked', storage: 'linked', link }
    } };
    svc.bind(1, project.id, { copied: { key: record.key }, linked: { link } });
    const snapshot = await svc.snapshotProject(project);
    expect(project.assets.copied).not.toHaveProperty('contentHash');
    const replacement = path.join(media, 'replacement.png');
    await settledFile(replacement, bytes(500, 2));
    await svc.store.putFile(replacement, { key: record.key });
    svc.bind(1, 'other-project', { copied: { hash: 'f'.repeat(64) } });
    await svc.release(1);
    await svc.bindJob(10, snapshot);
    expect(svc.jobSource(10, 'linked')).toEqual({ projectId: project.id, hash: link.hash });
    expect(svc.jobSource(10, 'copied')).toEqual({ projectId: project.id, hash: record.hash });
    expect(await svc.resolveAsset('copied', { process: 'job', session: 10 })).toMatchObject({ path: svc.store.objectPath(record.hash) });
    expect(await svc.resolveAsset('linked', { process: 'job', session: 10 })).toMatchObject({ path: file });
    const host = new HostServicesCore({ cacheDirectory: path.join(root, 'cache'), automaticOutputDirectory: path.join(root, 'out'),
      physicalMemory: 8 * 1024 ** 3, resolveAsset: (asset, context) => svc.resolveAsset(asset, context) });
    expect(await host.serve('job', 5, { type: 'mintHandle', kind: { kind: 'mediaRead', asset: 'copied' }, session: 10 }, 10))
      .toMatchObject({ type: 'handle', contentHash: Uint8Array.from(Buffer.from(record.hash, 'hex')) });
    // Interactive/session-number collisions and other jobs cannot borrow these bindings.
    expect(await svc.resolveAsset('copied', { process: 'interactive', session: 10 })).toBeNull();
    expect(await svc.resolveAsset('copied', { process: 'job', session: 11 })).toBeNull();
    await svc.bindJob(10, null);
    expect(await svc.resolveAsset('copied', { process: 'job', session: 10 })).toBeNull();
  });

  it('lets a waiting job resume when its media is put, relinked in its project window, or found by key', async () => {
    const svc = service();
    const gone = path.join(media, 'gone.mov');
    await settledFile(gone, bytes(2000, 3));
    const offlineLink = await svc.link(gone);
    await rm(gone);
    const keyed = path.join(media, 'keyed.png');
    await settledFile(keyed, bytes(800, 4));
    const keyedRecord = await svc.store.putFile(keyed, { key: 'media:v2:keyed' });
    const project = { id: 'waiting-project', assets: {
      // Copied media that was offline (no store record) when the job started.
      late: { id: 'late', storageKey: 'media:v2:late' },
      linked: { id: 'linked', storage: 'linked', link: offlineLink },
      // A saved content hash whose object is gone while its key still has bytes.
      keyed: { id: 'keyed', storageKey: keyedRecord.key, contentHash: blake3Hex(bytes(9, 9)) },
    } };
    await svc.bindJob(20, project);
    const job = { process: 'job' as const, session: 20 };
    expect(await svc.resolveAsset('late', job)).toBeNull();
    expect(await svc.resolveAsset('linked', job)).toBeNull();
    expect(await svc.resolveAsset('keyed', job)).toMatchObject({ path: svc.store.objectPath(keyedRecord.hash) });

    // wait_for_media re-mints every 2 s: the copy lands under the asset's key.
    const late = path.join(media, 'late.png');
    await settledFile(late, bytes(600, 5));
    const lateRecord = await svc.store.putFile(late, { key: 'media:v2:late' });
    expect(await svc.resolveAsset('late', job)).toMatchObject({ path: svc.store.objectPath(lateRecord.hash) });

    // Locate Media in a window: only a window bound to the job's project counts.
    const relinked = path.join(media, 'relinked.mov');
    await settledFile(relinked, bytes(2000, 6));
    const newLink = await svc.link(relinked);
    svc.bind(1, 'other-project', { linked: { link: newLink } });
    expect(await svc.resolveAsset('linked', job)).toBeNull();
    svc.bind(2, project.id, { linked: { link: newLink } });
    expect(await svc.resolveAsset('linked', job)).toMatchObject({ path: relinked });
    await svc.release(2);
    expect(await svc.resolveAsset('linked', job)).toBeNull();
  });

  it('gives desktop Save the store object only at the expected length', async () => {
    const svc = service();
    const file = path.join(media, 'x.bin');
    await settledFile(file, bytes(300));
    const record = await svc.store.putFile(file, { key: 'k' });
    expect(await svc.storedFile(record.hash, 300)).toBe(svc.store.objectPath(record.hash));
    expect(await svc.storedFile(record.hash, 299)).toBeNull();
    expect(await svc.storedFile('nothex', 300)).toBeNull();
  });
});

describe('still renditions of kept originals', () => {
  it('converts a stored or linked TIFF through a link that names its format, never touching the original', async () => {
    const seen: Array<{ named: string; resolved: string }> = [];
    const { realpath, readlink } = await import('node:fs/promises');
    const svc = service({
      convertStill: async (named) => {
        seen.push({ named: path.basename(named), resolved: await realpath(named) });
        expect(await readlink(named)).toBeTruthy();
        return { token: 'a'.repeat(32), size: 10 };
      }
    });
    const tiff = path.join(media, 'scan.tiff');
    await settledFile(tiff, bytes(700, 3));
    const stored = await svc.store.putFile(tiff, { key: 'media:v2:t' });
    expect(await svc.convertStill({ hash: stored.hash, name: 'original.tiff' })).toEqual({ ok: true, token: 'a'.repeat(32), size: 10, type: 'image/png' });
    expect(seen[0]).toEqual({ named: 'original.tiff', resolved: await realpath(svc.store.objectPath(stored.hash)) });
    const link = await svc.link(tiff);
    expect((await svc.convertStill({ link, name: 'original.tiff' })).ok).toBe(true);
    expect(seen[1]!.resolved).toBe(await realpath(tiff));
    // The temporary name is gone afterwards, and the original is untouched.
    const { readdir } = await import('node:fs/promises');
    expect((await readdir(path.join(svc.store.root, 'tmp'))).filter((name) => name.startsWith('still-'))).toEqual([]);
    expect(blake3Hex(await readFile(tiff))).toBe(stored.hash);
    // Formats the viewer decodes, and content that is nowhere, are refused.
    expect(await svc.convertStill({ hash: stored.hash, name: 'original.png' })).toMatchObject({ ok: false, error: /does not need converting/ });
    expect(await svc.convertStill({ hash: blake3Hex(bytes(5)), name: 'original.tiff' })).toMatchObject({ ok: false, error: /no longer available/ });
    expect(await service().convertStill({ hash: stored.hash, name: 'original.tiff' })).toMatchObject({ ok: false, error: /unavailable/ });
    // Content main does not have is reported as missing, so the page can send its bytes.
    expect(await svc.convertStill({ hash: blake3Hex(bytes(6)), name: 'original.tiff' })).toMatchObject({ missing: true });
  });

  it('converts a stored or linked video original into the viewer\'s playback proxy', async () => {
    const seen: string[] = [];
    const { realpath } = await import('node:fs/promises');
    const svc = service({ convertVideo: async (named) => { seen.push(`${path.basename(named)}→${await realpath(named)}`); return { token: 'b'.repeat(32), size: 20 }; } });
    const movie = path.join(media, 'A001.mov');
    await settledFile(movie, bytes(900, 5));
    const stored = await svc.store.putFile(movie, { key: 'media:v2:m' });
    expect(await svc.convertVideo({ hash: stored.hash, name: 'A001.mov' })).toEqual({ ok: true, token: 'b'.repeat(32), size: 20, type: 'video/webm' });
    expect(seen).toEqual([`original.mov→${await realpath(svc.store.objectPath(stored.hash))}`]);
    expect(await svc.convertVideo({ hash: stored.hash, name: 'clip.webm' })).toMatchObject({ ok: false, error: /not one Powermove can convert/ });
    expect(await svc.convertVideo({ hash: blake3Hex(bytes(4)), name: 'A001.mov' })).toMatchObject({ ok: false, missing: true });
    expect(await service().convertVideo({ hash: stored.hash, name: 'A001.mov' })).toMatchObject({ ok: false, error: /unavailable/ });
    const failing = service({ convertVideo: async () => { throw new Error('ffmpeg exited 1'); } });
    await failing.store.putFile(movie, { key: 'media:v2:m' });
    expect(await failing.convertVideo({ hash: stored.hash, name: 'A001.mov' })).toEqual({ ok: false, error: 'Could not optimize this video for playback · ffmpeg exited 1' });
  });

  it('converts uploaded bytes it does not have, verified by hash, and keeps none of them', async () => {
    const { readdir, readFile: read, realpath } = await import('node:fs/promises');
    const converted: Array<{ named: string; bytes: string }> = [];
    const convert = async (named: string) => { converted.push({ named: path.basename(named), bytes: blake3Hex(await read(await realpath(named))) }); return { token: 'c'.repeat(32), size: 30 }; };
    const svc = service({ enabled: false, convertStill: convert, convertVideo: convert });
    const upload = async (data: Uint8Array) => {
      const token = await svc.store.beginStream(7, { key: null, type: '', size: data.length });
      for (let at = 0; at < data.length; at += 4 * 1024 * 1024) await svc.store.chunk(7, token, at, data.subarray(at, at + 4 * 1024 * 1024));
      return token;
    };
    const tiff = bytes(5 * 1024 * 1024 + 9, 11);
    expect(await svc.convertUpload(7, await upload(tiff), { kind: 'still', name: 'original.tiff', blake3: blake3Hex(tiff) }))
      .toEqual({ ok: true, token: 'c'.repeat(32), size: 30, type: 'image/png' });
    const movie = bytes(4096, 12);
    expect(await svc.convertUpload(7, await upload(movie), { kind: 'video', name: 'A001.mov', blake3: blake3Hex(movie) })).toMatchObject({ ok: true, type: 'video/webm' });
    expect(converted).toEqual([{ named: 'original.tiff', bytes: blake3Hex(tiff) }, { named: 'original.mov', bytes: blake3Hex(movie) }]);
    // Bytes that are not what the page hashed are never converted.
    expect(await svc.convertUpload(7, await upload(movie), { kind: 'video', name: 'A001.mov', blake3: blake3Hex(tiff) })).toMatchObject({ ok: false, error: /does not match/ });
    // A refused conversion still discards the upload; another window's upload is not this one's.
    expect(await svc.convertUpload(7, await upload(movie), { kind: 'still', name: 'original.png', blake3: blake3Hex(movie) })).toMatchObject({ ok: false });
    const theirs = await upload(movie);
    await expect(svc.convertUpload(8, theirs, { kind: 'video', name: 'A001.mov', blake3: blake3Hex(movie) })).rejects.toThrow(/Unknown media upload/);
    await svc.store.abortStream(7, theirs);
    // The store kept nothing: no object, no key, no temporary file.
    expect(converted).toHaveLength(2);
    expect(await svc.store.records()).toEqual([]);
    expect(await readdir(path.join(svc.store.root, 'objects'))).toEqual([]);
    expect(await readdir(path.join(svc.store.root, 'tmp'))).toEqual([]);
  });
});

describe('media store IPC', () => {
  it('serves trusted windows only and reports the flag synchronously', async () => {
    const handlers = new Map<string, (...args: any[]) => any>();
    const ipc = { handle: (channel: string, fn: any) => handlers.set(channel, fn), on: (channel: string, fn: any) => handlers.set(channel, fn) };
    const svc = service({ enabled: true });
    let trusted = false;
    registerMediaStoreIpc(ipc as never, svc, { isTrustedSender: () => trusted });
    const sender = { id: 4, once: () => undefined };
    const event: any = { sender };
    await expect(Promise.resolve().then(() => handlers.get(MEDIA_STORE_IPC.record)!(event, 'k'))).rejects.toThrow(/Unauthorized/);
    handlers.get(MEDIA_STORE_IPC.config)!(event);
    expect(event.returnValue).toEqual({ enabled: false });
    trusted = true;
    handlers.get(MEDIA_STORE_IPC.config)!(event);
    expect(event.returnValue).toEqual({ enabled: true });
    expect(await handlers.get(MEDIA_STORE_IPC.record)!(event, 'k')).toBeNull();
    await expect(Promise.resolve().then(() => handlers.get(MEDIA_STORE_IPC.putFile)!(event, { path: 'relative', key: 'k' }))).rejects.toThrow(/absolute/);
    await expect(Promise.resolve().then(() => handlers.get(MEDIA_STORE_IPC.open)!(event, { link: { path: '/x', hash: 'zz', size: 1 } }))).rejects.toThrow(/Invalid media link/);
    await expect(Promise.resolve().then(() => handlers.get(MEDIA_STORE_IPC.convertUpload)!(event, { token: 'a'.repeat(32), kind: 'shell', name: 'x.mov', blake3: '' }))).rejects.toThrow(/Invalid conversion/);
    await expect(Promise.resolve().then(() => handlers.get(MEDIA_STORE_IPC.convertUpload)!(event, { token: '../x', kind: 'video', name: 'x.mov', blake3: '' }))).rejects.toThrow(/Invalid media token/);
    expect(await handlers.get(MEDIA_STORE_IPC.convertVideo)!(event, { hash: 'a'.repeat(64), name: 'x.mov' })).toMatchObject({ ok: false, error: /unavailable/ });
  });

  it('copies a range of the project file the window opened straight into the store', async () => {
    const handlers = new Map<string, (...args: any[]) => any>();
    const ipc = { handle: (channel: string, fn: any) => handlers.set(channel, fn), on: (channel: string, fn: any) => handlers.set(channel, fn) };
    const svc = service({ enabled: true });
    // A project file: 100 bytes of document, then the media range.
    const media = bytes(3 * 1024 * 1024 + 11, 9);
    const file = new Uint8Array(100 + media.length);
    file.set(media, 100);
    const sender = { id: 4, once: () => undefined };
    const other = { id: 5, once: () => undefined };
    const reads: number[] = [];
    registerMediaStoreIpc(ipc as never, svc, {
      isTrustedSender: () => true,
      projectRead: async (owner, token, offset, length) => {
        if (owner !== sender || token !== 'reader-1') throw new Error('Unknown project read token');
        reads.push(length);
        return file.slice(offset, offset + length);
      }
    });
    const call = (channel: string, value: unknown, from: object = sender) => Promise.resolve().then(() => handlers.get(channel)!({ sender: from }, value));
    const token = await call(MEDIA_STORE_IPC.beginPut, { key: 'media:v2:p', type: 'video/mp4', size: media.length, revision: 'rev-p', origin: 'project' });
    // Another window's reader, or a range past the limit, is refused.
    await expect(call(MEDIA_STORE_IPC.putProjectRange, { token, readToken: 'reader-1', offset: 100, length: 10, at: 0 }, other)).rejects.toThrow();
    await expect(call(MEDIA_STORE_IPC.putProjectRange, { token, readToken: 'reader-1', offset: 100, length: PROJECT_RANGE_BYTES + 1, at: 0 })).rejects.toThrow(/range/);
    await call(MEDIA_STORE_IPC.putProjectRange, { token, readToken: 'reader-1', offset: 100, length: 2 * 1024 * 1024, at: 0 });
    await call(MEDIA_STORE_IPC.putProjectRange, { token, readToken: 'reader-1', offset: 100 + 2 * 1024 * 1024, length: media.length - 2 * 1024 * 1024, at: 2 * 1024 * 1024 });
    expect(Math.max(...reads)).toBeLessThanOrEqual(1024 * 1024);
    const sha256 = createHash('sha256').update(media).digest('hex');
    const record = await call(MEDIA_STORE_IPC.finishPut, { token, sha256 });
    expect(record).toMatchObject({ key: 'media:v2:p', hash: blake3Hex(media), size: media.length, revision: 'rev-p', source: 'project' });
    expect(Buffer.compare(await readFile(svc.store.objectPath(record.hash)), media)).toBe(0);
  });
});
