import { installBridgeForTests, resetBridgeForTests } from '../../kernel/bridge';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { PMRegistry } from '../registry';
import { makePM } from '../__tests__/make-pm';

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
  resetBridgeForTests();
});

function posterRegistry(): {
  PM: PMRegistry;
  put: ReturnType<typeof vi.fn>;
  checkpoint: ReturnType<typeof vi.fn>;
  posterBlob: Blob;
} {
  let nextUrl = 0;
  const posterBlob = new Blob(['poster'], { type: 'image/webp' });
  const canvas: any = {
    width: 0, height: 0,
    getContext: () => ({ drawImage: vi.fn() }),
    toBlob: (callback: (blob: Blob) => void) => callback(posterBlob),
  };
  vi.stubGlobal('window', {
    File,
    document: { createElement: () => canvas },
    navigator: { hardwareConcurrency: 4 },
    createImageBitmap: vi.fn(async () => ({ width: 640, height: 360, close: vi.fn() })),
    URL: {
      createObjectURL: vi.fn(() => `blob:test-${++nextUrl}`),
      revokeObjectURL: vi.fn(),
    },
    setTimeout, clearTimeout,
  });
  const PM = makePM('core/history', 'core/media', 'gl/raster');
  PM.proj = { id: 'project-1', assets: {}, layers: [], comps: {} };
  PM.touch = vi.fn();
  PM.autosave = vi.fn();
  const checkpoint = vi.fn();
  PM.Projects = { put: checkpoint };
  PM.MediaImport.fingerprint = vi.fn(async () => 'fingerprint');
  const put = vi.fn(async () => true);
  PM.MediaStore = { put, get: vi.fn(async () => null) };
  return { PM, put, checkpoint, posterBlob };
}

function imageFile(name = 'photo.png') {
  return Object.assign(new Blob(['image'], { type: 'image/png' }), { name });
}

describe('raster media posters', () => {
  it('persists an imported poster beside the source and records it in asset metadata', async () => {
    const { PM, put, posterBlob } = posterRegistry();
    const file = imageFile();

    const asset = await PM.assets.add(file);

    expect(put).toHaveBeenNthCalledWith(1, 'media:fingerprint', file, expect.objectContaining({ storageKey: 'media:fingerprint' }));
    expect(put).toHaveBeenNthCalledWith(2, 'media:fingerprint:poster', posterBlob, {
      storageKey: 'media:fingerprint:poster', type: 'image/webp',
    });
    expect(PM.proj.assets[asset.id]).toMatchObject({ storageKey: 'media:fingerprint', poster: true });
    expect(PM.assets.poster(asset.id)).toBe('blob:test-2');
    PM.MediaImport.removeAsset(PM.proj, asset.id);
    expect(PM.assets.poster(asset.id)).toBe('');
    expect(window.URL.revokeObjectURL).toHaveBeenCalledWith('blob:test-2');
    PM.assets.clear();
  });

  it('restores a poster even when the source media bytes are missing', async () => {
    const { PM, posterBlob } = posterRegistry();
    const meta = {
      id: 'offline-image', name: 'offline.png', kind: 'image',
      storageKey: 'media:offline', poster: true,
    };
    PM.proj.assets[meta.id] = meta;
    PM.MediaStore.get = vi.fn(async (value: any) =>
      value === 'media:offline:poster' ? posterBlob : null);

    const result = await PM.assets.restoreProject(PM.proj);

    expect(result.missing).toEqual([meta]);
    expect(PM.assets.get(meta.id)).toBeUndefined();
    expect(PM.assets.poster(meta.id)).toBe('blob:test-1');
    PM.assets.clear();
    expect(window.URL.revokeObjectURL).toHaveBeenCalledWith('blob:test-1');
  });

  it('repairs missing poster metadata when the stored poster exists', async () => {
    const { PM, checkpoint, posterBlob } = posterRegistry();
    const meta: any = {
      id: 'unflagged-image', name: 'unflagged.png', kind: 'image',
      storageKey: 'media:unflagged',
    };
    PM.proj.assets[meta.id] = meta;
    PM.MediaStore.get = vi.fn(async (value: any) =>
      value === 'media:unflagged:poster' ? posterBlob : imageFile(meta.name));

    const result = await PM.assets.restoreProject(PM.proj);

    expect(result.restored).toHaveLength(1);
    expect(PM.MediaStore.get).toHaveBeenCalledWith('media:unflagged:poster');
    expect(PM.assets.poster(meta.id)).not.toBe('');
    expect(meta.poster).toBe(true);
    expect(PM.touch).toHaveBeenCalledOnce();
    expect(checkpoint).toHaveBeenCalledWith(PM.proj);
    PM.assets.clear();
  });

  it('backfills a poster when neither metadata nor a stored poster exists', async () => {
    vi.useFakeTimers();
    const { PM, put } = posterRegistry();
    const meta: any = {
      id: 'posterless-image', name: 'posterless.png', kind: 'image',
      storageKey: 'media:posterless',
    };
    PM.proj.assets[meta.id] = meta;
    PM.MediaStore.get = vi.fn(async (value: any) =>
      typeof value === 'object' ? imageFile(meta.name) : null);

    const result = await PM.assets.restoreProject(PM.proj);
    expect(result.restored).toHaveLength(1);
    expect(PM.MediaStore.get).toHaveBeenCalledWith('media:posterless:poster');
    expect(meta.poster).toBeUndefined();

    await vi.runAllTimersAsync();

    expect(put).toHaveBeenCalledWith('media:posterless:poster', expect.any(Blob), {
      storageKey: 'media:posterless:poster', type: 'image/webp',
    });
    expect(meta.poster).toBe(true);
    expect(PM.assets.poster(meta.id)).not.toBe('');
    PM.assets.clear();
  });

  it('refreshes the poster when replacing an asset in place', async () => {
    const { PM, put } = posterRegistry();
    const id = 'replace-image';
    PM.proj.assets[id] = {
      id, name: 'old.png', kind: 'image', storageKey: 'media:old', persisted: true,
    };
    PM.assets.map.set(id, {
      id, name: 'old.png', kind: 'image', w: 100, h: 100,
      el: { width: 100, height: 100, close: vi.fn() },
    });

    const result = await PM.assets.replace(id, imageFile('new.png'));

    expect(result.meta).toMatchObject({ id, name: 'new.png', poster: true });
    expect(PM.proj.assets[id].poster).toBe(true);
    expect(PM.assets.poster(id)).toBe('blob:test-2');
    expect(put.mock.calls.map(([key]) => key)).toEqual(['media:fingerprint', 'media:fingerprint:poster']);
    PM.hist.clear();
    PM.assets.clear();
  });

  it('lazily backfills restored legacy assets and emits one assets event for the batch', async () => {
    vi.useFakeTimers();
    const { PM, put } = posterRegistry();
    const first: any = { id: 'first', name: 'first.png', kind: 'image', storageKey: 'media:first' };
    const second: any = { id: 'second', name: 'second.png', kind: 'image', storageKey: 'media:second' };
    PM.proj.assets = { first, second };
    PM.MediaStore.get = vi.fn(async (value: any) => typeof value === 'object' ? imageFile(value.name) : null);
    const assetsEvent = vi.fn();
    PM.bus.on('assets', assetsEvent);

    const result = await PM.assets.restoreProject(PM.proj);
    expect(result.restored).toHaveLength(2);
    expect(first.poster).toBeUndefined();
    assetsEvent.mockClear();

    await vi.runAllTimersAsync();

    expect(first.poster).toBe(true);
    expect(second.poster).toBe(true);
    expect(put).toHaveBeenCalledTimes(2);
    expect(assetsEvent).toHaveBeenCalledOnce();
    expect(PM.assets.poster('first')).not.toBe('');
    expect(PM.assets.poster('second')).not.toBe('');
    PM.assets.clear();
  });
});

it('publishes ready media while another restore waits, and clears loading on failure', async () => {
  const { PM } = posterRegistry();
  let release!: () => void;
  const pending = new Promise<void>(resolve => { release = resolve; });
  PM.proj.assets = {
    fast: { id: 'fast', name: 'fast.png', kind: 'image' },
    slow: { id: 'slow', name: 'slow.png', kind: 'image' },
  };
  PM.MediaStore.get = async (meta: any) => {
    if (meta.id === 'slow') { await pending; throw new Error('Read failed'); }
    return imageFile(meta.name);
  };
  const restore = PM.assets.restoreProject(PM.proj);
  expect([...PM.assets.loading]).toEqual(['fast', 'slow']);
  await vi.waitFor(() => expect(PM.assets.get('fast')).toBeDefined());
  expect([...PM.assets.loading]).toEqual(['slow']);
  release();
  expect((await restore).missing.map((m: any) => m.id)).toEqual(['slow']);
  expect(PM.assets.loading.size).toBe(0);
  PM.assets.clear();
});

function localSourceRegistry() {
  const registry = posterRegistry();
  const { PM } = registry;
  const meta = { id: 'local-image', kind: 'image', name: 'photo.png', sourcePath: '/photos/photo.png', fingerprint: 'fingerprint', storageKey: 'media:fingerprint' };
  PM.proj.assets[meta.id] = meta;
  const media = {
    openLocalSource: vi.fn(async () => ({ token: 'local', size: 5 })),
    readCloudSource: vi.fn(async () => new TextEncoder().encode('image')),
    releaseCloudSource: vi.fn(async () => undefined),
  };
  (window as any).powermove = { media };
  installBridgeForTests((window as any).powermove);
  return { ...registry, meta, media };
}

it.each(['missing', 'read error', 'decode error'])('recovers the original file when the media cache has a %s', async failure => {
  const { PM, meta, media, put } = localSourceRegistry();
  PM.proj.layers = [{ id: 'clip', type: 'image', d: { asset: meta.id }, from: 2, to: 4 }];
  const layers = JSON.stringify(PM.proj.layers);
  if (failure === 'read error') PM.MediaStore.get.mockRejectedValue(new Error('Cache read failed'));
  if (failure === 'decode error') {
    PM.MediaStore.get.mockImplementation(async (value: any) => typeof value === 'object' ? imageFile() : null);
    vi.mocked(window.createImageBitmap).mockRejectedValueOnce(new Error('Corrupt cache'));
  }
  const result = await PM.assets.restoreProject(PM.proj);
  expect(result.missing).toEqual([]);
  expect(result.restored).toHaveLength(1);
  expect(PM.assets.get(meta.id)).toMatchObject({ id: meta.id, storageKey: meta.storageKey, persisted: true });
  expect(JSON.stringify(PM.proj.layers)).toBe(layers);
  expect(PM.proj.assets[meta.id]).toBe(meta);
  expect(put).toHaveBeenCalledWith(meta.storageKey, expect.any(Blob), expect.objectContaining({ fingerprint: 'fingerprint' }));
  expect(media.releaseCloudSource).toHaveBeenCalledWith('local');
  expect(PM.assets.errors.size).toBe(0);
  expect(PM.assets.loading.size).toBe(0);
  PM.assets.clear();
});

it('rejects a changed source instead of silently replacing existing timeline media', async () => {
  const { PM, meta, put } = localSourceRegistry();
  PM.MediaImport.fingerprint.mockResolvedValue('different-file');
  const original = JSON.stringify(meta);
  const result = await PM.assets.restoreProject(PM.proj);
  expect(result.missing).toEqual([meta]);
  expect(PM.assets.get(meta.id)).toBeUndefined();
  expect(PM.assets.errors.get(meta.id)).toContain('source file has changed');
  expect(JSON.stringify(meta)).toBe(original);
  expect(put).not.toHaveBeenCalled();
  PM.assets.clear();
});

it('keeps loading visible until source recovery finishes and discards recovery after a project switch', async () => {
  const { PM, meta, media, put } = localSourceRegistry();
  let finish!: (value: { token: string; size: number }) => void;
  media.openLocalSource.mockImplementation(() => new Promise(resolve => { finish = resolve; }));
  const restore = PM.assets.restoreProject(PM.proj);
  await vi.waitFor(() => expect(media.openLocalSource).toHaveBeenCalled());
  expect(PM.assets.loading.has(meta.id)).toBe(true);
  PM.assets.clear();
  PM.proj = { assets: {}, layers: [] };
  finish({ token: 'local', size: 5 });
  expect(await restore).toMatchObject({ stale: true });
  expect(PM.assets.map.size).toBe(0);
  expect(PM.assets.errors.size).toBe(0);
  expect(media.releaseCloudSource).toHaveBeenCalledWith('local');
  expect(put).not.toHaveBeenCalled();
});
