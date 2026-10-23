import { describe, expect, it, vi } from 'vitest';
import { BlenderPngs, type BlenderPngHost } from './blender-png';
import { sha256Hex } from './svg-rasters';
import type { CompatFrameRequest } from '../../../shared/engine-protocol';

const original = new Uint8Array([0, 0, 0, 28, 102, 116, 121, 112, 97, 118, 105, 102, 1]);
const encoded = new Uint8Array([137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0, 0, 0, 0, 0]);

async function request(bytes = original, asset = 'a1'): Promise<CompatFrameRequest> {
  const content = (await sha256Hex(bytes)).match(/../g)!.map(h => parseInt(h, 16));
  return { document: 'doc', composition: 'c', layer: 'L', time: 0, source: 'blenderPng', scale: 1,
    revision: 1, blender: { asset, content: new Uint8Array(content) } } as CompatFrameRequest;
}

describe('Blender Canvas2D PNG transport', () => {
  it('returns the exact toBlob bytes and caches only after matching original content', async () => {
    const drawImage = vi.fn();
    const toBlob = vi.fn((done: (blob: Blob) => void) => done(new Blob([encoded])));
    const canvas = { width: 0, height: 0, getContext: () => ({ drawImage }), toBlob } as unknown as HTMLCanvasElement;
    const host: BlenderPngHost = {
      meta: () => ({ kind: 'image', name: 'map.avif' }),
      blob: async () => new Blob([original]),
      image: () => ({ el: { width: 1, height: 1 } as CanvasImageSource }),
      canvas: () => canvas,
      decode: vi.fn(),
    };
    const pngs = new BlenderPngs(host);
    const ask = await request();
    const one = await pngs.produce(ask);
    const two = await pngs.produce(ask);
    expect(one).toMatchObject({ key: await sha256Hex(original), width: 1, height: 1, format: 'png' });
    expect(one.data).toEqual(encoded);
    expect(two.data).toEqual(encoded);
    expect(toBlob).toHaveBeenCalledTimes(1);
    expect(drawImage).toHaveBeenCalledTimes(1);
    await expect(pngs.produce(await request(new Uint8Array([9])))).rejects.toThrow(/changed/);
  });

  it('evicts least-recently-used successful PNG bytes while preserving a recent hit', async () => {
    const sources = new Map([
      ['a', new Uint8Array([1])], ['b', new Uint8Array([2])], ['c', new Uint8Array([3])]
    ]);
    const toBlob = vi.fn((done: (blob: Blob) => void) => done(new Blob([encoded])));
    const canvas = { width: 0, height: 0, getContext: () => ({ drawImage: vi.fn() }), toBlob } as unknown as HTMLCanvasElement;
    const host: BlenderPngHost = {
      meta: id => ({ kind: 'image', name: id }),
      blob: async meta => new Blob([sources.get(meta.name)!]),
      image: () => ({ el: { width: 1, height: 1 } as CanvasImageSource }),
      canvas: () => canvas,
      decode: vi.fn(),
    };
    const pngs = new BlenderPngs(host, encoded.byteLength * 2);
    const ask = async (id: 'a' | 'b' | 'c') => pngs.produce(await request(sources.get(id)!, id));
    await ask('a'); await ask('b'); await ask('a'); await ask('c');
    expect(toBlob).toHaveBeenCalledTimes(3);
    await ask('a');
    expect(toBlob).toHaveBeenCalledTimes(3);
    await ask('b');
    expect(toBlob).toHaveBeenCalledTimes(4);
  });

  it('returns an oversize PNG without retaining it and retries failed conversions', async () => {
    let fail = true;
    const toBlob = vi.fn((done: (blob: Blob | null) => void) => done(fail ? null : new Blob([encoded])));
    const canvas = { width: 0, height: 0, getContext: () => ({ drawImage: vi.fn() }), toBlob } as unknown as HTMLCanvasElement;
    const host: BlenderPngHost = {
      meta: () => ({ kind: 'image', name: 'map.avif' }), blob: async () => new Blob([original]),
      image: () => ({ el: { width: 1, height: 1 } as CanvasImageSource }), canvas: () => canvas,
      decode: vi.fn(),
    };
    const pngs = new BlenderPngs(host, encoded.byteLength - 1);
    const ask = await request();
    await expect(pngs.produce(ask)).rejects.toThrow(/convert texture/);
    fail = false;
    expect((await pngs.produce(ask)).data).toEqual(encoded);
    expect((await pngs.produce(ask)).data).toEqual(encoded);
    expect(toBlob).toHaveBeenCalledTimes(3);
  });

  it('coalesces concurrent requests for one original hash', async () => {
    let release!: () => void;
    const decoding = new Promise<void>(resolve => { release = resolve; });
    const decode = vi.fn(async () => { await decoding; return { width: 1, height: 1, close: vi.fn() } as unknown as ImageBitmap; });
    const toBlob = vi.fn((done: (blob: Blob) => void) => done(new Blob([encoded])));
    const canvas = { width: 0, height: 0, getContext: () => ({ drawImage: vi.fn() }), toBlob } as unknown as HTMLCanvasElement;
    const host: BlenderPngHost = {
      meta: () => ({ kind: 'image', name: 'map.avif' }), blob: async () => new Blob([original]),
      image: () => null, canvas: () => canvas, decode,
    };
    const pngs = new BlenderPngs(host, encoded.byteLength);
    const ask = await request();
    const first = pngs.produce(ask), second = pngs.produce(ask);
    await vi.waitFor(() => expect(decode).toHaveBeenCalledTimes(1));
    release();
    const [a, b] = await Promise.all([first, second]);
    expect(a.data).toEqual(encoded);
    expect(b.data).toEqual(encoded);
    expect(toBlob).toHaveBeenCalledTimes(1);
  });
});
