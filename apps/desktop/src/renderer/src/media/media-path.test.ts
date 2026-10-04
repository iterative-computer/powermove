// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { installBridgeForTests, resetBridgeForTests } from '../kernel/bridge';
import { MEDIA_STAGE_CHUNK_BYTES } from '../../../shared/media-tools';
import { resetMediaPathForTests, resolveMediaPath, resolveMediaSource } from './media-path';

type Listener = () => void;

function registry(assets: Record<string, any>, blobs: Record<string, Blob | null> = {}) {
  const listeners: Record<string, Listener[]> = {};
  const PM = {
    proj: { id: 'P1', assets },
    assets: { get: (id: string) => (assets[id]?.loaded ? { dur: 4, playbackProxy: assets[id].playbackProxy } : undefined), errors: new Map<string, string>() },
    MediaStore: { get: vi.fn(async (meta: any) => blobs[meta.id] ?? null) },
    bus: { on: (name: string, cb: Listener) => { (listeners[name] ??= []).push(cb); }, emit: (name: string) => listeners[name]?.forEach((cb) => cb()) }
  };
  (window as any).PM = PM;
  return PM;
}

function mediaBridge(overrides: Record<string, any> = {}) {
  const staged: Uint8Array[] = [];
  const media = {
    lookup: vi.fn(async () => null),
    stageBegin: vi.fn(async () => ({ token: 't1' })),
    stageChunk: vi.fn(async (_token: string, _offset: number, data: Uint8Array) => { staged.push(data); }),
    stageFinish: vi.fn(async () => ({ path: '/cache/a1.mov', origin: 'cache' as const })),
    stageAbort: vi.fn(async () => {}),
    release: vi.fn(async () => {}),
    ...overrides
  };
  installBridgeForTests({ mediaPath: media } as never);
  return { media, staged };
}

beforeEach(() => resetMediaPathForTests());
afterEach(() => { resetBridgeForTests(); delete (window as any).PM; });

describe('resolveMediaPath', () => {
  it('uses the verified original source without reading any bytes', async () => {
    const PM = registry({ a1: { id: 'a1', name: 'Interview.mov', kind: 'video', sourcePath: '/Volumes/Shoot/Interview.mov', fingerprint: 'v2:10:abc', storageKey: 'media:v2:10:abc', size: 10, loaded: true } });
    const { media } = mediaBridge({ lookup: vi.fn(async () => ({ path: '/Volumes/Shoot/Interview.mov', origin: 'source' })) });
    expect(await resolveMediaPath('a1')).toBe('/Volumes/Shoot/Interview.mov');
    expect(media.lookup).toHaveBeenCalledWith({ assetId: 'a1', projectId: 'P1', sourcePath: '/Volumes/Shoot/Interview.mov', fingerprint: 'v2:10:abc', storageKey: 'media:v2:10:abc', size: 10 });
    expect(PM.MediaStore.get).not.toHaveBeenCalled();
    expect(media.stageBegin).not.toHaveBeenCalled();
  });

  it('stages the stored bytes in ordered chunks when the source is gone, once for concurrent callers', async () => {
    const blob = new Blob([new Uint8Array(MEDIA_STAGE_CHUNK_BYTES + 5).fill(3)], { type: 'video/webm' });
    registry({ a1: { id: 'a1', name: 'Clip.mov', kind: 'video', storageKey: 'media:k', playbackProxy: true, loaded: true } }, { a1: blob });
    const { media, staged } = mediaBridge();
    const [first, second] = await Promise.all([resolveMediaSource('a1'), resolveMediaSource('a1')]);
    expect(first).toEqual({ path: '/cache/a1.mov', origin: 'cache', assetId: 'a1', name: 'Clip.mov', kind: 'video', proxy: true });
    expect(second).toBe(first);
    expect(media.stageBegin).toHaveBeenCalledTimes(1);
    expect(media.stageBegin).toHaveBeenCalledWith({ assetId: 'a1', name: 'Clip.mov', size: blob.size, projectId: 'P1', storageKey: 'media:k', type: 'video/webm' });
    expect(media.stageChunk.mock.calls.map(([, offset]) => offset)).toEqual([0, MEDIA_STAGE_CHUNK_BYTES]);
    expect(staged.reduce((sum, chunk) => sum + chunk.length, 0)).toBe(blob.size);
  });

  it('ignores a single frame of an image sequence as its source', async () => {
    registry({ s1: { id: 's1', name: 'seq', kind: 'video', sourcePath: '/frames/0001.png', fingerprint: 'v2:1:a', imageSequence: { fps: 24, frames: 10 } } }, { s1: new Blob(['x']) });
    const { media } = mediaBridge();
    await resolveMediaPath('s1');
    expect((media.lookup.mock.calls[0] as unknown as [Record<string, unknown>])[0]).not.toHaveProperty('sourcePath');
  });

  it('hands the bytes to the web host with its upload path', async () => {
    registry({ a1: { id: 'a1', name: 'Clip.mp4', kind: 'audio', storageKey: 'media:k' } }, { a1: new Blob(['abc']) });
    const stageFile = vi.fn(async (file: File) => ({ path: `/host/${file.name}`, origin: 'host' as const }));
    const { media } = mediaBridge({ stageFile });
    expect(await resolveMediaPath('a1')).toBe('/host/Clip.mp4');
    expect(media.stageBegin).not.toHaveBeenCalled();
  });

  it('rejects unknown assets, stills and media with no bytes', async () => {
    registry({ i1: { id: 'i1', name: 'Still.png', kind: 'image' }, v1: { id: 'v1', name: 'Gone.mov', kind: 'video' } });
    mediaBridge();
    await expect(resolveMediaPath('nope')).rejects.toThrow(/does not exist/);
    await expect(resolveMediaPath('i1')).rejects.toThrow(/an image asset/);
    await expect(resolveMediaPath('v1')).rejects.toThrow(/no media bytes.*Locate File/);
  });

  it('aborts a failed staging and lets main drop a closed project\'s files', async () => {
    const PM = registry({ a1: { id: 'a1', name: 'Clip.mov', kind: 'video' } }, { a1: new Blob(['abc']) });
    const { media } = mediaBridge({ stageChunk: vi.fn(async () => { throw new Error('disk full'); }) });
    await expect(resolveMediaPath('a1')).rejects.toThrow('disk full');
    expect(media.stageAbort).toHaveBeenCalledWith('t1');
    PM.bus.emit('projects:open');
    expect(media.release).not.toHaveBeenCalled();
    PM.proj = { id: 'P2', assets: {} } as never;
    PM.bus.emit('projects:open');
    expect(media.release).toHaveBeenCalledWith('P1');
  });
});
