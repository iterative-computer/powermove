import { createHash } from 'node:crypto';
import { afterEach, expect, it, vi } from 'vitest';
import { attachLibraryMedia } from './library-media';
import type { LibraryMediaBridge } from '../../../shared/library-media';

afterEach(() => vi.unstubAllGlobals());
const sha = (value: string) => createHash('sha256').update(value).digest('hex');
const record = { revision: 'revision', size: 5, type: 'video/mp4', url: 'app://powermove/_media/token', sha256: sha('video') };
function setup() {
  const host = { get: vi.fn(async () => ({ ...record })), link: vi.fn(async (input: any) => ({ ...record, sourcePath: input.sourcePath })),
    begin: vi.fn(async () => 'upload'), chunk: vi.fn(async () => {}), finish: vi.fn(async () => ({ ...record })), abort: vi.fn(async () => {}), remove: vi.fn(async () => true) };
  const legacy = { record: vi.fn(async (_value: unknown): Promise<any> => null), keys: vi.fn(async () => ['clip']), removeIfUnchanged: vi.fn(async () => true) };
  vi.stubGlobal('fetch', vi.fn(async () => new Response(new Blob(['video'], { type: 'video/mp4' }))));
  return { host, legacy, store: attachLibraryMedia(host as LibraryMediaBridge, legacy) };
}

it('links originals without uploading their contents and keeps pasted media as owned bytes', async () => {
  const { store, host } = setup();
  expect(await store.put('clip', new Blob(['video'], { type: 'video/mp4' }), { sourcePath: '/media/video.mp4' })).toBe(true);
  expect(host.link).toHaveBeenCalledWith(expect.objectContaining({ key: 'clip', sourcePath: '/media/video.mp4', size: 5 }));
  expect(host.begin).not.toHaveBeenCalled();
  expect(await store.put('paste', new Blob(['video'], { type: 'video/mp4' }))).toBe(true);
  expect(host.chunk).toHaveBeenCalled();
});

it('keeps links out of ordinary saves but still reads the bytes for portable exports', async () => {
  const { store, host } = setup();
  const asset = { id: 'clip', sourcePath: '/media/video.mp4' };
  expect(await store.getForSave(asset)).toEqual({ linked: true, revision: 'revision' });
  expect(fetch).not.toHaveBeenCalled();
  expect(await (await store.get(asset))!.text()).toBe('video');
  expect(host.link).toHaveBeenCalledWith(expect.objectContaining({ matchExisting: true, revision: 'revision' }));
});

it('removes a byte-identical browser copy even when old migration revision labels differ', async () => {
  const { store, legacy } = setup();
  const old = { id: 'clip', revision: 'older-revision', blob: new Blob(['video']) };
  legacy.record.mockResolvedValue(old);
  expect(await store.removeLegacyDuplicates()).toBe(1);
  expect(legacy.removeIfUnchanged).toHaveBeenCalledWith(old);
  legacy.removeIfUnchanged.mockClear(); legacy.record.mockResolvedValue({ ...old, blob: new Blob(['other']) });
  expect(await store.removeLegacyDuplicates()).toBe(0);
  expect(legacy.removeIfUnchanged).not.toHaveBeenCalled();
});

it('keeps a valid owned copy if its original moved or changed', async () => {
  const { store, host } = setup();
  host.link.mockRejectedValue(new Error('Source does not match'));
  const saved = await store.getForSave({ id: 'clip', sourcePath: '/media/changed.mp4' });
  expect(saved?.linked).toBeUndefined();
  expect(saved?.source).toMatchObject({ size: 5, type: 'video/mp4', sha256: sha('video') });
  expect(fetch).not.toHaveBeenCalled();
});

it('can save edits with a missing linked source without reading or embedding it', async () => {
  const { store, host } = setup();
  host.get.mockImplementation(async (_key?: any, metadataOnly?: any) => {
    if (!metadataOnly) throw new Error('Use Locate File');
    return { ...record, sourcePath: '/media/moved.mp4' };
  });
  expect(await store.getForSave({ id: 'clip', sourcePath: '/media/moved.mp4' })).toEqual({ linked: true, revision: 'revision' });
  expect(fetch).not.toHaveBeenCalled();
});

it('upgrade preserves different legacy bytes and only removes verified duplicates', async () => {
  const { store, host, legacy } = setup();
  const old = { id: 'clip', revision: 'older-revision', blob: new Blob(['video']) };
  legacy.record.mockResolvedValue(old);
  await store.optimizeLegacy();
  expect(legacy.removeIfUnchanged).toHaveBeenCalledWith(old); expect(host.begin).not.toHaveBeenCalled();
  legacy.removeIfUnchanged.mockClear(); legacy.record.mockResolvedValue({ ...old, blob: new Blob(['other']) });
  await store.optimizeLegacy();
  expect(legacy.removeIfUnchanged).not.toHaveBeenCalled(); expect(host.begin).not.toHaveBeenCalled();
});

it('upgrade uploads browser-only media durably before retiring its old record', async () => {
  const { store, host, legacy } = setup();
  host.get.mockResolvedValue(null as any);
  const old = { id: 'clip', revision: 'older-revision', blob: new Blob(['video']) };
  legacy.record.mockResolvedValue(old);
  const order: string[] = [];
  host.finish.mockImplementation(async () => { order.push('durable'); return { ...record }; });
  legacy.removeIfUnchanged.mockImplementation(async () => { order.push('remove'); return true; });
  await store.optimizeLegacy();
  expect(order).toEqual(['durable', 'remove']);
});

it('upgrade leaves the old browser record intact on upload failure or an unavailable native source', async () => {
  const { store, host, legacy } = setup();
  const old = { id: 'clip', blob: new Blob(['video']) }; legacy.record.mockResolvedValue(old);
  host.get.mockImplementation(async (_key?: any, metadataOnly?: boolean) => {
    if (metadataOnly) return { ...record };
    throw new Error('The original moved');
  });
  await store.optimizeLegacy(); expect(legacy.removeIfUnchanged).not.toHaveBeenCalled();
  host.get.mockResolvedValue(null as any); host.finish.mockRejectedValue(new Error('Disk full'));
  await expect(store.optimizeLegacy()).rejects.toThrow('Disk full'); expect(legacy.removeIfUnchanged).not.toHaveBeenCalled();
});
