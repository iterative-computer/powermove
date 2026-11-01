import { afterEach, expect, it, vi } from 'vitest';
import { StreamedMedia, isStreamedMedia } from './streamed-media';

const FILE = Uint8Array.from({ length: 3 * 1024 * 1024 + 7 }, (_, index) => index % 251);

function serveFile() {
  const requests: Array<string | null> = [];
  vi.stubGlobal('fetch', vi.fn(async (_url: string, init?: RequestInit) => {
    const range = (init?.headers as Record<string, string> | undefined)?.Range ?? null;
    requests.push(range);
    if (!range) return new Response(FILE.slice());
    const [, start, end] = /^bytes=(\d+)-(\d*)$/.exec(range)!;
    const last = end === '' ? FILE.length - 1 : Number(end);
    if (end !== '' && last - Number(start) + 1 > 1024 * 1024) return new Response(null, { status: 416 });
    return new Response(FILE.slice(Number(start), last + 1), { status: 206 });
  }));
  return requests;
}

afterEach(() => vi.unstubAllGlobals());

it('reads a whole file with one plain request and a tail with one open-ended range', async () => {
  const requests = serveFile();
  const media = new StreamedMedia('app://powermove/_media/clip', FILE.length, 'video/webm');
  expect(new Uint8Array(await media.arrayBuffer())).toEqual(FILE);
  expect(new Uint8Array(await media.slice(FILE.length - 10).arrayBuffer())).toEqual(FILE.slice(-10));
  expect(requests).toEqual([null, `bytes=${FILE.length - 10}-`]);
});

it('reads interior slices with bounded ranges of at most one MiB, like Blob.slice', async () => {
  const requests = serveFile();
  const media = new StreamedMedia('app://powermove/_media/clip', FILE.length);
  expect(new Uint8Array(await media.slice(16, 48).arrayBuffer())).toEqual(FILE.slice(16, 48));
  expect(new Uint8Array(await media.slice(5, 5 + 2.5 * 1024 * 1024).arrayBuffer())).toEqual(FILE.slice(5, 5 + 2.5 * 1024 * 1024));
  expect(requests[0]).toBe('bytes=16-47');
  expect(requests.slice(1)).toEqual([`bytes=5-${1024 * 1024 + 4}`, `bytes=${1024 * 1024 + 5}-${2 * 1024 * 1024 + 4}`, `bytes=${2 * 1024 * 1024 + 5}-${2.5 * 1024 * 1024 + 4}`]);
  // Nested and negative slices clamp exactly as Blob does.
  const nested = media.slice(100, 200).slice(-10);
  expect(nested.size).toBe(10);
  expect(new Uint8Array(await nested.arrayBuffer())).toEqual(FILE.slice(190, 200));
  expect(media.slice(10, 5).size).toBe(0);
  expect((await media.slice(10, 5).arrayBuffer()).byteLength).toBe(0);
});

it('fails instead of returning partial bytes', async () => {
  vi.stubGlobal('fetch', vi.fn(async () => new Response(new Uint8Array(3), { status: 206 })));
  await expect(new StreamedMedia('app://x/_media/a', 100).slice(0, 10).arrayBuffer()).rejects.toThrow('incomplete');
  vi.stubGlobal('fetch', vi.fn(async () => new Response(null, { status: 404 })));
  await expect(new StreamedMedia('app://x/_media/a', 100).arrayBuffer()).rejects.toThrow('could not be read');
  expect(isStreamedMedia(new StreamedMedia('a', 1))).toBe(true);
  expect(isStreamedMedia(new Blob(['a']))).toBe(false);
});
