import { installBridgeForTests, resetBridgeForTests } from '../../kernel/bridge';
import { afterEach, expect, it, vi } from 'vitest';
import { createCloudMedia, cloudSourcePaths, readLocalMediaSource } from './cloud-media';

afterEach(() => vi.unstubAllGlobals());
function setup(automatic = false) {
  const meta = { id: 'a', name: 'clip.mp4', sourcePath: '/cloud/clip.mp4' };
  const project = { assets: { a: meta } };
  const media = {
    cloudStatus: vi.fn(async () => ({ '/cloud/clip.mp4': 'icloud' })),
    openLocalSource: vi.fn(async (): Promise<{ token: string; size: number } | null> => ({ token: 'local-token', size: 5 })),
    cloudPrompt: vi.fn(async () => ({ download: false, automatic: false })),
    downloadCloudSource: vi.fn(async () => ({ token: 'token', size: 5 })),
    readCloudSource: vi.fn(async () => new Uint8Array([1, 2, 3, 4, 5])),
    releaseCloudSource: vi.fn(async () => undefined),
  };
  vi.stubGlobal('window', { powermove: { media }, File });
  installBridgeForTests((window as any).powermove);
  const PM = {
    proj: project, assets: { get: vi.fn() }, bus: { emit: vi.fn() }, toast: vi.fn(), invalidate: vi.fn(),
    store: { get: vi.fn(() => automatic), set: vi.fn() },
    MediaImport: { mapBounded: async (items: any[], _limit: number, worker: any) => Promise.all(items.map(worker)) },
  };
  const recover = vi.fn(async (_meta: any, _file: File, _current: () => boolean) => undefined);
  const cloud = createCloudMedia(PM, recover);
  return { PM, project, meta, media, recover, cloud };
}

it('detects cloud media without reading it, prompts once, and respects Not now', async () => {
  const { cloud, project, meta, media, PM } = setup();
  await cloud.scan(project, [meta]);
  expect(cloud.get('a')).toEqual({ provider: 'iCloud', state: 'offloaded' });
  expect(media.cloudPrompt).toHaveBeenCalledWith(['clip.mp4']);
  expect(media.downloadCloudSource).not.toHaveBeenCalled();
  expect(PM.store.set).toHaveBeenCalledWith('autoDownloadCloudMedia', false);
  await cloud.scan(project, [meta]);
  expect(media.cloudPrompt).toHaveBeenCalledTimes(1);
});
it('keeps deleted and unknown files out of the cloud prompt', async () => {
  const { cloud, project, meta, media } = setup();
  media.cloudStatus.mockResolvedValue({ '/cloud/clip.mp4': 'missing' });
  await cloud.scan(project, [meta]);
  expect(cloud.get('a')).toBeUndefined();
  expect(media.cloudPrompt).not.toHaveBeenCalled();
});
it('saves the checkbox and restores the same asset after an accepted download', async () => {
  const { cloud, project, meta, media, PM, recover } = setup();
  media.cloudPrompt.mockResolvedValue({ download: true, automatic: true });
  await cloud.scan(project, [meta]);
  await vi.waitFor(() => expect(recover).toHaveBeenCalledTimes(1));
  expect(PM.store.set).toHaveBeenCalledWith('autoDownloadCloudMedia', true);
  const [restoredMeta, file] = recover.mock.calls[0]!;
  expect(restoredMeta).toBe(meta);
  expect(file.name).toBe(meta.name);
  expect(file.size).toBe(5);
  expect(cloudSourcePaths.get(file)).toBe(meta.sourcePath);
  expect(media.releaseCloudSource).toHaveBeenCalledWith('token');
  expect(cloud.get('a')).toBeUndefined();
});
it('automatically downloads without asking when the saved preference is enabled', async () => {
  const { cloud, project, meta, media, recover } = setup(true);
  await cloud.scan(project, [meta]);
  await vi.waitFor(() => expect(recover).toHaveBeenCalledTimes(1));
  expect(media.cloudPrompt).not.toHaveBeenCalled();
});
it('deduplicates manual downloads and allows retry after failure', async () => {
  const { cloud, project, meta, media, recover } = setup();
  await cloud.scan(project, [meta]);
  media.downloadCloudSource.mockRejectedValueOnce(new Error('Network unavailable'));
  const first = cloud.download('a');
  expect(cloud.download('a')).toBe(first);
  await first;
  expect(cloud.get('a')).toMatchObject({ state: 'error', error: 'Network unavailable' });
  await cloud.download('a');
  expect(recover).toHaveBeenCalledTimes(1);
  expect(cloud.get('a')).toBeUndefined();
});
it('releases downloaded bytes without touching a project that was closed', async () => {
  const { cloud, project, meta, media, recover, PM } = setup();
  await cloud.scan(project, [meta]);
  let finish!: (value: { token: string; size: number }) => void;
  media.downloadCloudSource.mockImplementation(() => new Promise(resolve => { finish = resolve; }));
  const download = cloud.download('a');
  cloud.clear();
  PM.proj = { assets: { a: { ...meta } } };
  finish({ token: 'token', size: 5 });
  await download;
  expect(media.readCloudSource).not.toHaveBeenCalled();
  expect(media.releaseCloudSource).toHaveBeenCalledWith('token');
  expect(recover).not.toHaveBeenCalled();
  expect(cloud.get('a')).toBeUndefined();
});
it('does not apply an old prompt answer to another project', async () => {
  const { cloud, project, meta, media, PM } = setup();
  let answer!: (value: { download: boolean; automatic: boolean }) => void;
  media.cloudPrompt.mockImplementation(() => new Promise(resolve => { answer = resolve; }));
  await cloud.scan(project, [meta]);
  cloud.clear();
  PM.proj = { assets: { a: { ...meta } } };
  answer({ download: true, automatic: true });
  await Promise.resolve();
  expect(media.downloadCloudSource).not.toHaveBeenCalled();
  expect(PM.store.set).not.toHaveBeenCalled();
});
it('releases tokens after a read failure and retains a retry action', async () => {
  const { cloud, project, meta, media, recover } = setup();
  await cloud.scan(project, [meta]);
  media.readCloudSource.mockRejectedValueOnce(new Error('Read failed'));
  await cloud.download('a');
  expect(media.releaseCloudSource).toHaveBeenCalledWith('token');
  expect(recover).not.toHaveBeenCalled();
  expect(cloud.get('a')).toMatchObject({ state: 'error' });
});

it('reads a local source and preserves its path without prompting or downloading', async () => {
  const { meta, media } = setup();
  const file = await readLocalMediaSource(meta, () => true);
  expect([...new Uint8Array(await file!.arrayBuffer())]).toEqual([1, 2, 3, 4, 5]);
  expect(cloudSourcePaths.get(file!)).toBe(meta.sourcePath);
  expect(media.releaseCloudSource).toHaveBeenCalledWith('local-token');
  expect(media.downloadCloudSource).not.toHaveBeenCalled();
  expect(media.cloudPrompt).not.toHaveBeenCalled();
});

it('skips nonlocal sources and supports legacy saved paths', async () => {
  const { media } = setup();
  media.openLocalSource.mockResolvedValue(null);
  expect(await readLocalMediaSource({ id: 'a', name: 'clip.mp4', path: '/old/clip.mp4' }, () => true)).toBeNull();
  expect(media.openLocalSource).toHaveBeenCalledWith('/old/clip.mp4');
  expect(media.readCloudSource).not.toHaveBeenCalled();
});

it('releases local source handles when the project changes while opening', async () => {
  const { meta, media } = setup();
  let current = true;
  media.openLocalSource.mockImplementation(async () => { current = false; return { token: 'local-token', size: 5 }; });
  expect(await readLocalMediaSource(meta, () => current)).toBeNull();
  expect(media.releaseCloudSource).toHaveBeenCalledWith('local-token');
  expect(media.readCloudSource).not.toHaveBeenCalled();
});

it('releases local source handles after a truncated read', async () => {
  const { meta, media } = setup();
  media.readCloudSource.mockResolvedValue(new Uint8Array());
  await expect(readLocalMediaSource(meta, () => true)).rejects.toThrow('ended unexpectedly');
  expect(media.releaseCloudSource).toHaveBeenCalledWith('local-token');
});
