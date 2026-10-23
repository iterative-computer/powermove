import { expect, it, vi } from 'vitest';
import { MEDIA_STORE_IPC } from '../../../shared/media-store';
import { hiddenMediaStore } from './hidden-media-store';

it('uses the authenticated host store and waits for verified upload completion', async () => {
  let finish!: (record: unknown) => void;
  const invoke = vi.fn((channel: string) => channel === MEDIA_STORE_IPC.finishPut
    ? new Promise(resolve => { finish = resolve; }) : Promise.resolve('upload'));
  const store = hiddenMediaStore({ invoke: invoke as never });
  expect(store.configSync()).toEqual({ enabled: true });
  await store.beginPut({ key: 'media:input', type: 'image/svg+xml', size: 4 });
  const bytes = new Uint8Array([1, 2, 3, 4]); await store.putChunk('upload', 0, bytes);
  let committed = false;
  const pending = store.finishPut('upload', { blake3: 'a'.repeat(64) }).then(record => { committed = true; return record; });
  await Promise.resolve(); expect(committed).toBe(false);
  const record = { key: 'media:input', hash: 'a'.repeat(64), size: 4 };
  finish(record); expect(await pending).toEqual(record);
  expect(invoke.mock.calls).toEqual([
    [MEDIA_STORE_IPC.beginPut, { key: 'media:input', type: 'image/svg+xml', size: 4 }],
    [MEDIA_STORE_IPC.putChunk, { token: 'upload', offset: 0, data: bytes }],
    [MEDIA_STORE_IPC.finishPut, { token: 'upload', blake3: 'a'.repeat(64) }]
  ]);
});
