import { EventEmitter } from 'node:events';
import { mkdir, mkdtemp, readFile, readdir, realpath, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { afterEach, describe, expect, it } from 'vitest';

import { MEDIA_PATH_IPC, MEDIA_STAGE_CHUNK_BYTES } from '../shared/media-tools';
import { MediaPathCache, mediaFingerprint, parseLookupRequest, parseStageRequest, registerMediaPathIpc } from './media-path-cache';

const roots: string[] = [];
async function temp(): Promise<string> {
  const root = await mkdtemp(path.join(os.tmpdir(), 'powermove-media-path-'));
  roots.push(root);
  return root;
}
afterEach(async () => { await Promise.all(roots.splice(0).map((root) => rm(root, { recursive: true, force: true }))); });

/** The renderer's own import fingerprint, for parity. */
async function rendererFingerprint(bytes: Uint8Array): Promise<string> {
  // The renderer module is outside main's TypeScript project; load it by path.
  const module = '../renderer/src/legacy/core/media';
  const { install } = await import(/* @vite-ignore */ module) as { install(PM: unknown): void };
  const PM: Record<string, any> = {};
  install(PM);
  return PM.MediaImport.fingerprint(new Blob([bytes as BlobPart]));
}

function bytes(size: number, seed = 7): Uint8Array {
  const data = new Uint8Array(size);
  for (let index = 0; index < size; index++) data[index] = (index * 31 + seed) & 0xff;
  return data;
}

describe('media fingerprint', () => {
  it('matches the renderer import fingerprint for small and sampled files', async () => {
    const root = await temp();
    for (const size of [10, 64 * 1024, 64 * 1024 * 3 + 17]) {
      const data = bytes(size);
      const file = path.join(root, `f-${size}`);
      await writeFile(file, data);
      expect(await mediaFingerprint(file)).toBe(await rendererFingerprint(data));
    }
  });
});

describe('MediaPathCache', () => {
  it('uses the original source only while it still matches the import fingerprint', async () => {
    const root = await temp();
    const source = path.join(root, 'Interview.mov');
    const data = bytes(200_000);
    await writeFile(source, data);
    const fingerprint = await rendererFingerprint(data);
    const cache = new MediaPathCache(path.join(root, 'cache'));
    expect(await cache.lookup(1, { assetId: 'a1', sourcePath: source, fingerprint })).toEqual({ path: await realpath(source), origin: 'source' });

    const changed = bytes(200_000, 9);
    await writeFile(source, changed);
    expect(await cache.lookup(1, { assetId: 'a1', sourcePath: source, fingerprint })).toBeNull();
    expect(await cache.lookup(1, { assetId: 'a1', sourcePath: path.join(root, 'missing.mov'), fingerprint })).toBeNull();
    expect(await cache.lookup(1, { assetId: 'a1', sourcePath: source })).toBeNull();
  });

  it('never reads a cloud placeholder as a source', async () => {
    const root = await temp();
    const source = path.join(root, 'clip.mp4');
    const data = bytes(1000);
    await writeFile(source, data);
    const cache = new MediaPathCache(path.join(root, 'cache'), { isLocal: async () => false });
    expect(await cache.lookup(1, { assetId: 'a1', sourcePath: source, fingerprint: await rendererFingerprint(data) })).toBeNull();
  });

  it('finds the host copy by store key on powermove serve', async () => {
    const root = await temp();
    const store = path.join(root, 'Media Store');
    await mkdir(store);
    await writeFile(path.join(store, 'media_v2_10_abc'), 'x');
    const cache = new MediaPathCache(path.join(root, 'cache'), { mediaStoreDir: store });
    expect(await cache.lookup(1, { assetId: 'a1', storageKey: 'media:v2:10:abc' })).toEqual({ path: path.join(store, 'media_v2_10_abc'), origin: 'host' });
    expect(await cache.lookup(1, { assetId: 'a1', storageKey: 'media:v2:10:other' })).toBeNull();
  });

  it('stages bytes once in ordered chunks, then serves the cached file', async () => {
    const root = await temp();
    const cache = new MediaPathCache(path.join(root, 'cache'));
    const data = bytes(MEDIA_STAGE_CHUNK_BYTES + 100);
    const request = { assetId: 'a1', projectId: 'P1', storageKey: 'media:v2:x', name: 'clip.mov', type: 'video/webm', size: data.length };
    const begun = await cache.begin(3, request);
    if (!('token' in begun)) throw new Error('expected a staging token');
    await expect(cache.chunk(3, begun.token, 5, data.subarray(0, 10))).rejects.toThrow(/Invalid/);
    await expect(cache.chunk(4, begun.token, 0, data.subarray(0, 10))).rejects.toThrow(/Unknown/);
    await cache.chunk(3, begun.token, 0, data.subarray(0, MEDIA_STAGE_CHUNK_BYTES));
    await cache.chunk(3, begun.token, MEDIA_STAGE_CHUNK_BYTES, data.subarray(MEDIA_STAGE_CHUNK_BYTES));
    const staged = await cache.finish(3, begun.token);
    expect(staged.origin).toBe('cache');
    expect(path.extname(staged.path)).toBe('.webm');
    expect(new Uint8Array(await readFile(staged.path))).toEqual(data);

    expect(await cache.lookup(3, { assetId: 'a1', storageKey: 'media:v2:x', size: data.length })).toEqual(staged);
    expect(await cache.begin(5, request)).toEqual(staged);
  });

  it('rejects an incomplete staging and leaves nothing behind', async () => {
    const root = await temp();
    const cache = new MediaPathCache(path.join(root, 'cache'));
    const begun = await cache.begin(1, { assetId: 'a1', name: 'a.wav', size: 20 });
    if (!('token' in begun)) throw new Error('expected a staging token');
    await cache.chunk(1, begun.token, 0, bytes(10));
    await expect(cache.finish(1, begun.token)).rejects.toThrow(/Incomplete/);
    expect(await readdir(path.join(root, 'cache'))).toEqual([]);
  });

  it('drops a project\'s files when its window closes it, keeping files another window uses', async () => {
    const root = await temp();
    const cache = new MediaPathCache(path.join(root, 'cache'));
    const stage = async (owner: number, projectId: string, assetId: string) => {
      const begun = await cache.begin(owner, { assetId, projectId, name: 'a.wav', size: 4 });
      if (!('token' in begun)) return begun;
      await cache.chunk(owner, begun.token, 0, bytes(4));
      return cache.finish(owner, begun.token);
    };
    const shared = await stage(1, 'P1', 'shared');
    await stage(2, 'P2', 'shared');
    const own = await stage(1, 'P1', 'own');
    await cache.release(1, 'P1');
    await expect(readFile(own.path)).rejects.toThrow();
    expect(await readFile(shared.path)).toBeTruthy();
    await cache.release(2);
    await expect(readFile(shared.path)).rejects.toThrow();
    await cache.dispose();
    await expect(readdir(path.join(root, 'cache'))).rejects.toThrow();
  });

  it('stops a copy still in flight when its project is closed, so nothing outlives the project', async () => {
    const root = await temp();
    const cache = new MediaPathCache(path.join(root, 'cache'));
    const closing = await cache.begin(1, { assetId: 'big', projectId: 'P1', name: 'a.wav', size: 8 });
    const staying = await cache.begin(1, { assetId: 'other', projectId: 'P2', name: 'b.wav', size: 4 });
    if (!('token' in closing) || !('token' in staying)) throw new Error('expected staging tokens');
    await cache.chunk(1, closing.token, 0, bytes(4));
    await cache.release(1, 'P1');
    await expect(cache.chunk(1, closing.token, 4, bytes(4))).rejects.toThrow(/Unknown/);
    await expect(cache.finish(1, closing.token)).rejects.toThrow();
    await cache.chunk(1, staying.token, 0, bytes(4));
    const kept = await cache.finish(1, staying.token);
    expect(await readdir(path.join(root, 'cache'))).toEqual([path.basename(kept.path)]);
    await cache.dispose();
  });
});

describe('media path IPC', () => {
  it('validates payloads', () => {
    expect(() => parseLookupRequest({ assetId: '../x y' })).toThrow(/asset id/);
    expect(parseLookupRequest({ assetId: 'a1', sourcePath: 'relative/path', fingerprint: 'nope', storageKey: 'bad key', size: -1 })).toEqual({ assetId: 'a1' });
    expect(() => parseStageRequest({ assetId: 'a1', name: 'x', size: 0 })).toThrow(/size/);
  });

  it('serves trusted windows and releases a window\'s files when it goes away', async () => {
    const root = await temp();
    const handlers = new Map<string, (event: unknown, value?: unknown) => unknown>();
    const ipc = { handle: (channel: string, handler: (event: unknown, value?: unknown) => unknown) => { handlers.set(channel, handler); } };
    const cache = new MediaPathCache(path.join(root, 'cache'));
    registerMediaPathIpc(ipc as never, cache, { isTrustedSender: (event: any) => event.trusted === true });
    expect([...handlers.keys()].sort()).toEqual(Object.values(MEDIA_PATH_IPC).sort());
    const sender = Object.assign(new EventEmitter(), { id: 9, isDestroyed: () => false });
    const event = { trusted: true, sender };
    await expect(Promise.resolve().then(() => handlers.get(MEDIA_PATH_IPC.lookup)!({ trusted: false, sender }, { assetId: 'a1' }))).rejects.toThrow(/Unauthorized/);
    const begun = await handlers.get(MEDIA_PATH_IPC.stageBegin)!(event, { assetId: 'a1', projectId: 'P1', name: 'a.wav', size: 3 }) as { token: string };
    await handlers.get(MEDIA_PATH_IPC.stageChunk)!(event, { token: begun.token, offset: 0, data: bytes(3) });
    const staged = await handlers.get(MEDIA_PATH_IPC.stageFinish)!(event, begun.token) as { path: string };
    expect(await readFile(staged.path)).toHaveLength(3);
    sender.emit('destroyed');
    await new Promise((resolve) => setTimeout(resolve, 20));
    await expect(readFile(staged.path)).rejects.toThrow();
  });
});
