import { afterEach, expect, it, vi } from 'vitest';
import { mediaRangeSource } from './media-range';

const record = { revision: 'revision', size: 5, type: 'video/mp4', url: 'app://powermove/_media/test' };
afterEach(() => vi.unstubAllGlobals());
const response = (body: BodyInit | null, headers = { 'Content-Range': 'bytes 1-3/5', 'Content-Length': '3' }) => new Response(body, { status: 206, headers });

it('requests and consumes only the requested range with the caller cancellation signal', async () => {
  const fetcher = vi.fn(async () => response('ide')); vi.stubGlobal('fetch', fetcher);
  const signal = new AbortController().signal;
  expect([...await mediaRangeSource(record).read(1, 3, signal)]).toEqual([...new TextEncoder().encode('ide')]);
  expect(fetcher).toHaveBeenCalledWith(record.url, expect.objectContaining({ signal, headers: { Range: 'bytes=1-3' } }));
  await expect(mediaRangeSource(record).read(0, 2 * 1024 * 1024)).rejects.toThrow('Invalid media range');
  expect(fetcher).toHaveBeenCalledTimes(1);
});

it('cancels an ignored range before consuming the full response', async () => {
  const cancel = vi.fn();
  vi.stubGlobal('fetch', vi.fn(async () => new Response(new ReadableStream({ cancel }), { status: 200 })));
  await expect(mediaRangeSource(record).read(1, 3)).rejects.toThrow('HTTP 200');
  expect(cancel).toHaveBeenCalledOnce();
});

it.each(['truncated', 'oversized', 'wrong range'])('rejects %s media responses', async failure => {
  vi.stubGlobal('fetch', vi.fn(async () => failure === 'wrong range'
    ? response('ide', { 'Content-Range': 'bytes 0-2/5', 'Content-Length': '3' })
    : response(failure === 'truncated' ? 'id' : 'idea')));
  await expect(mediaRangeSource(record).read(1, 3)).rejects.toThrow();
});

it('does not start a fetch after cancellation', async () => {
  const fetcher = vi.fn(); vi.stubGlobal('fetch', fetcher);
  const controller = new AbortController(); controller.abort();
  await expect(mediaRangeSource(record).read(1, 3, controller.signal)).rejects.toMatchObject({ name: 'AbortError' });
  expect(fetcher).not.toHaveBeenCalled();
});

it('cancels an active body read and rejects with the caller abort reason', async () => {
  const controller = new AbortController(), cancel = vi.fn();
  vi.stubGlobal('fetch', vi.fn(async () => response(new ReadableStream({ cancel }))));
  const reading = mediaRangeSource(record).read(1, 3, controller.signal);
  const rejected = expect(reading).rejects.toMatchObject({ name: 'AbortError' });
  await Promise.resolve(); await Promise.resolve();
  controller.abort();
  await rejected; expect(cancel).toHaveBeenCalledOnce();
});
