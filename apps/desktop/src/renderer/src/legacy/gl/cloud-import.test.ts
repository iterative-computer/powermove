import { afterEach, expect, it, vi } from 'vitest';
import { installBridgeForTests, resetBridgeForTests } from '../../kernel/bridge';
import { install } from './raster';

afterEach(() => { vi.unstubAllGlobals(); resetBridgeForTests(); });

it('imports fresh cloud bytes before fingerprinting, then lets the next batch run after failure', async () => {
  const source = '/cloud/voice.wav';
  const stale = new File([], 'voice.wav', { type: 'audio/wav' });
  const media = {
    sourcePath: () => source,
    cloudStatus: vi.fn(async () => ({ [source]: 'cloud' })),
    downloadCloudSource: vi.fn(async () => ({ token: 'download', size: 3 })),
    readCloudSource: vi.fn(async () => new Uint8Array([1, 2, 3])),
    releaseCloudSource: vi.fn(async () => {}),
  };
  vi.stubGlobal('window', { File, navigator: { hardwareConcurrency: 2 }, document: { createElement: () => ({ getContext: () => ({}) }) } });
  installBridgeForTests({ media } as any);
  let next = 0;
  const fingerprint = vi.fn(async (file: File) => {
    expect(file).not.toBe(stale);
    expect(file.type).toBe('audio/wav');
    expect([...new Uint8Array(await file.arrayBuffer())]).toEqual([1, 2, 3]);
    return 'fresh';
  });
  const PM: any = {
    uid: () => `asset-${++next}`, proj: { id: 'project', assets: {}, layers: [] },
    bus: { emit: vi.fn() }, touch: vi.fn(),
    Audio: { accepts: () => true, prepareAsset: async ({ id, name, blob }: any) => ({ id, name, kind: 'audio', size: blob.size }), rebalanceCache: vi.fn() },
    MediaStore: { put: vi.fn(async () => true) },
    MediaImport: { fingerprint, storageKeyFor: () => 'media:fresh', match: () => ({ canonicalId: null, aliases: [] }), coalesce: () => 0,
      mapBounded: async (files: File[], _limit: number, action: any) => Promise.all(files.map(action)) },
  };
  install(PM);
  const stages: string[] = [];
  const [result] = await PM.assets.importBatch([stale], { onStage: ({ label }: any) => stages.push(label) });
  expect(result.status).toBe('created');
  expect(result.persisted).toBe(true);
  expect(result.asset.sourcePath).toBe(source);
  expect(stages.indexOf('Downloading from cloud…')).toBeLessThan(stages.indexOf('Reading file'));
  expect(fingerprint).toHaveBeenCalledTimes(1);
  media.downloadCloudSource.mockRejectedValueOnce(new Error('Network unavailable'));
  const [failed] = await PM.assets.importBatch([stale]);
  expect(failed.status).toBe('failed');
  expect(failed.error.message).toBe('Network unavailable');
  expect(fingerprint).toHaveBeenCalledTimes(1);
  expect((await PM.assets.importBatch([stale]))[0].status).toBe('created');
  expect(media.releaseCloudSource).toHaveBeenCalledTimes(2);
});
