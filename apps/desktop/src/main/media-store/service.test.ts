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
    await svc.release(2);
    expect(await svc.resolveAsset('a1')).not.toBeNull();
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
