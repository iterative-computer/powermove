import { mkdtemp, rm, readFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { expect, it, vi } from 'vitest';
import { TransitionMedia } from '../src/main/transition/media';
import { InProcessExecutor } from '../src/main/media-store/hasher';
import { prepareTransition } from '../src/renderer/src/host/transition';
import { blake3Hex } from '../src/shared/blake3';
const storage = (values: Record<string, string>) => ({ get length() { return Object.keys(values).length; }, key: (i: number) => Object.keys(values)[i] ?? null, getItem: (key: string) => values[key] ?? null }) as Storage;
it('moves every Blob and poster, preserves metadata-only records and resumes without re-uploading', async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), 'pm-tr-renderer-'));
  try {
    const service = new TransitionMedia({ root, executor: new InProcessExecutor(), freeBytes: async () => 20 * 1024 ** 3 });
    const host = { request: vi.fn((command: any) => service.request(42, command)) };
    const entries = [{ locator: 'one', key: 'media:one', value: { id: 'one', blob: new Blob(['original']), poster: new Blob(['poster']), unknown: 'retained' } },
      { locator: 'opaque', key: 'opaque', value: { unknown: ['value'] } }];
    const progress = vi.fn();
    await prepareTransition(host, async () => entries, storage({ unknown: 'exact' }), progress);
    expect(await service.request(42, { action: 'status' })).toMatchObject({ mediaComplete: true, done: 2, localStorageComplete: true, cutoverReady: false });
    expect(progress).toHaveBeenLastCalledWith(2, 2);
    host.request.mockClear();
    await prepareTransition(host, async () => entries, storage({ unknown: 'exact' }), progress);
    expect(host.request.mock.calls.filter(([c]) => c.action === 'begin')).toHaveLength(0);
    expect((await service.store.records()).length).toBe(2);
    const record = await service.store.record('media:one');
    expect(record?.hash).toBe(blake3Hex(new TextEncoder().encode('original')));
    expect((await readFile(path.join(root, 'migration.json'), 'utf8'))).toContain('retained');
  } finally { await rm(root, { recursive: true, force: true }); }
});
