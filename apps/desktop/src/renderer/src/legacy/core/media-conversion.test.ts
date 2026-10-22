import { afterEach, expect, it, vi } from 'vitest';
import { installBridgeForTests, resetBridgeForTests } from '../../kernel/bridge';
import { readProxyFile } from './media-conversion';

afterEach(() => { resetBridgeForTests(); vi.unstubAllGlobals(); });

function mediaHost(readPlaybackProxy: any) {
  vi.stubGlobal('window', { File });
  const media = { readPlaybackProxy, releasePlaybackProxy: vi.fn(async () => {}) };
  installBridgeForTests({ media } as any);
  return media;
}

it('copies converted files incrementally, accounting for short reads and reporting bytes', async () => {
  const media = mediaHost(vi.fn(async (_token, offset) => new Uint8Array(offset === 0 ? [1, 2, 3] : [4, 5, 6])));
  const onStage = vi.fn();
  const file = await readProxyFile({ token: 'owned', size: 6, type: 'image/png' }, 'still.png', 123, { onStage });
  expect([...new Uint8Array(await file.arrayBuffer())]).toEqual([1, 2, 3, 4, 5, 6]);
  expect(media.readPlaybackProxy.mock.calls.map((call: any[]) => call[1])).toEqual([0, 3]);
  expect(onStage).toHaveBeenLastCalledWith('Loading converted media', { completed: 6, total: 6 });
  expect(media.releasePlaybackProxy).toHaveBeenCalledExactlyOnceWith('owned');
});

it('stops copying on cancellation and releases temporary native output', async () => {
  const media = mediaHost(vi.fn(async () => new Uint8Array([1]))), controller = new AbortController();
  await expect(readProxyFile({ token: 'cancelled', size: 100 }, 'movie.webm', 1, {
    assertCurrent: () => controller.signal.throwIfAborted(), onStage: () => controller.abort(),
  })).rejects.toMatchObject({ name: 'AbortError' });
  expect(media.readPlaybackProxy).toHaveBeenCalledOnce();
  expect(media.releasePlaybackProxy).toHaveBeenCalledExactlyOnceWith('cancelled');
});

it.each(['truncated', 'read failed'])('releases native output after %s', async failure => {
  const media = mediaHost(vi.fn(async () => {
    if (failure === 'read failed') throw new Error('Read failed');
    return new Uint8Array();
  }));
  await expect(readProxyFile({ token: 'failed', size: 2 }, 'movie.webm', 1)).rejects.toThrow();
  expect(media.releasePlaybackProxy).toHaveBeenCalledExactlyOnceWith('failed');
});
