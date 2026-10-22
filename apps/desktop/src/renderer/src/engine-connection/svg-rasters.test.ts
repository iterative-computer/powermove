// @vitest-environment happy-dom
/*
 * Level 1 SVG rasters for the engine (decision S1): the window draws a
 * bucket only from an asset whose decoded bytes hash to the engine's
 * content, posts it in bands, once per key in flight, and names why when
 * it cannot. Chromium's rasterization itself (the <img> into a canvas and
 * the texture read back) is compared with the legacy captures by
 * engine/crates/pm-render/tools/svg-rasters and the app's e2e: vitest has
 * no WebGL.
 */
import { describe, expect, it, vi } from 'vitest';

import type { SvgRasterRequest } from '../../../shared/engine-protocol';
import { MAX_ASK_LOG, MAX_SVG_BAND_BYTES, SvgRasterCanvas, SvgRasters, sha256Hex, sha256HexSync, type SvgAsset } from './svg-rasters';

const text = new TextEncoder();
const svgA = text.encode('<svg xmlns="http://www.w3.org/2000/svg" width="4" height="2"/>');
const svgB = text.encode('<svg xmlns="http://www.w3.org/2000/svg" width="4" height="3"/>');

async function setup() {
  const el = {} as CanvasImageSource;
  const assets = new Map<string, SvgAsset & { bytes: Uint8Array }>([
    ['a1', { el, url: 'blob:a1', format: 'svg', kind: 'image', bytes: svgA }],
    ['png', { el, url: 'blob:png', format: 'png', kind: 'image', bytes: new Uint8Array(4) }]
  ]);
  const reads: string[] = [];
  const draw = vi.fn((_img: CanvasImageSource, w: number, h: number) => new Uint8Array(w * h * 4).fill(w % 251));
  const rasters = new SvgRasters({
    asset: (id) => assets.get(id) ?? null,
    assets: () => assets.values(),
    bytes: async (asset) => {
      reads.push(asset.url!);
      return (asset as { bytes: Uint8Array }).bytes;
    },
    bytesSync: (asset) => {
      reads.push(`sync ${asset.url!}`);
      return (asset as { bytes: Uint8Array }).bytes;
    },
    canvas: { draw, drawTexture: (image: CanvasImageSource, w: number, h: number) => draw(image, w, h).fill(17) } as unknown as SvgRasterCanvas
  });
  const posted: Record<string, any>[] = [];
  const target = { post: (e: Record<string, unknown>) => { posted.push(e); return true; } };
  return { rasters, assets, reads, draw, posted, target, contentA: await sha256Hex(svgA), contentB: await sha256Hex(svgB) };
}

const request = (content: string, width = 4, height = 2, asset = 'a1'): SvgRasterRequest => ({ key: { content, width, height }, asset });

describe('SvgRasters', () => {
  it('hashes the bytes the asset decoded and draws the bucket asked for', async () => {
    const { rasters, draw, posted, target, contentA } = await setup();
    expect(contentA).toMatch(/^[0-9a-f]{64}$/);
    await rasters.answer(target, [request(contentA, 6, 3)]);
    expect(draw).toHaveBeenCalledOnce();
    expect(draw.mock.calls[0]!.slice(1)).toEqual([6, 3]);
    expect(rasters.asks).toEqual([{ asset: 'a1', width: 6, height: 3 }]);
    expect(posted).toHaveLength(1);
    expect(posted[0]).toMatchObject({ type: 'compat', compat: 'svgPixels', key: { content: contentA, width: 6, height: 3 }, row: 0, rows: 3 });
    expect((posted[0]!['data'] as Uint8Array).byteLength).toBe(6 * 3 * 4);
  });

  it('keeps a material-map upload distinct from a canvas bucket at the same size', async () => {
    const { rasters, posted, target, contentA } = await setup();
    const bucket = request(contentA), map = { ...bucket, key: { ...bucket.key, texture: true } };
    await Promise.all([rasters.answer(target, [bucket]), rasters.answer(target, [map])]);
    expect(posted).toHaveLength(2);
    expect(posted.find(p => p['key'].texture)?.['data'][0]).toBe(17);
    expect(posted.find(p => !p['key'].texture)?.['data'][0]).toBe(4);
    const produced = await rasters.produce({ svg: map } as any);
    expect(produced.key).toBe(`${contentA}:4x2:texture`);
    expect(produced.format).toBe('rgba8Straight');
    expect(rasters.drawSync(map)[0]).toBe(17);
  });

  it('never draws another version of the file under the engine’s content', async () => {
    const { rasters, draw, posted, target, contentB } = await setup();
    await rasters.answer(target, [request(contentB), request(contentB, 8, 4, 'gone')]);
    expect(draw).not.toHaveBeenCalled();
    expect(posted.map((p) => `${p['compat']}:${p['reason']}`).sort()).toEqual(['svgFailed:missing', 'svgFailed:stale']);
    expect(rasters.stats).toMatchObject({ failed: 2, stale: 1, drawn: 0 });
  });

  it('a replaced asset (a new object or source) is hashed again', async () => {
    const { rasters, assets, reads, target, contentA, contentB } = await setup();
    await rasters.answer(target, [request(contentA)]);
    await rasters.answer(target, [request(contentA, 8, 4)]);
    expect(reads).toEqual(['blob:a1']);
    assets.set('a1', { el: {} as CanvasImageSource, url: 'blob:a1-2', format: 'svg', kind: 'image', bytes: svgB });
    expect(await rasters.content(assets.get('a1')!)).toBe(contentB);
    expect(reads).toEqual(['blob:a1', 'blob:a1-2']);
  });

  it('draws each key once while it is in flight', async () => {
    const { rasters, draw, target, contentA } = await setup();
    await Promise.all([rasters.answer(target, [request(contentA)]), rasters.answer(target, [request(contentA)])]);
    expect(draw).toHaveBeenCalledOnce();
  });

  it('keeps the latest viewer asks, bounded', async () => {
    const { rasters, target, contentA } = await setup();
    for (let i = 1; i <= MAX_ASK_LOG + 3; i++) await rasters.answer(target, [request(contentA, i, 1)]);
    expect(rasters.asks).toHaveLength(MAX_ASK_LOG);
    expect(rasters.asks[0]).toEqual({ asset: 'a1', width: 4, height: 1 });
    expect(rasters.asks.at(-1)).toEqual({ asset: 'a1', width: MAX_ASK_LOG + 3, height: 1 });
  });

  it('posts large rasters in bands that fit a message', async () => {
    const { rasters, posted, target, contentA } = await setup();
    const width = 4096, height = 1024;
    await rasters.answer(target, [request(contentA, width, height)]);
    const rowsPerBand = Math.floor(MAX_SVG_BAND_BYTES / (width * 4));
    expect(posted.map((p) => [p['row'], p['rows']])).toEqual([[0, rowsPerBand], [rowsPerBand, height - rowsPerBand]]);
    expect(posted.every((p) => (p['data'] as Uint8Array).byteLength <= MAX_SVG_BAND_BYTES)).toBe(true);
    await rasters.answer(target, [request(contentA, 9000, 2)]);
    expect(posted.at(-1)).toMatchObject({ compat: 'svgFailed', reason: 'size' });
  });

  it('draws on the calling thread with the hash taken ahead, or taken there', async () => {
    const { rasters, assets, reads, contentA, contentB } = await setup();
    rasters.prime();
    await rasters.content(assets.get('a1')!);
    expect(rasters.drawSync(request(contentA)).byteLength).toBe(32);
    expect(reads).toEqual(['blob:a1']);
    // Replaced in the same task as the snapshot: hashed on this thread,
    // once, and still never drawn under another content.
    assets.set('a1', { el: {} as CanvasImageSource, url: 'blob:a1-2', format: 'svg', kind: 'image', bytes: svgB });
    expect(() => rasters.drawSync(request(contentA))).toThrow(/is not the engine's/);
    expect(rasters.drawSync(request(contentB)).byteLength).toBe(32);
    expect(reads).toEqual(['blob:a1', 'sync blob:a1-2']);
    expect(rasters.stats.hashedSync).toBe(1);
    expect(await rasters.content(assets.get('a1')!)).toBe(contentB);
    expect(reads).toHaveLength(2);
  });

  it('hashes on the calling thread exactly as crypto.subtle does', async () => {
    const lengths = [0, 1, 3, 55, 56, 63, 64, 65, 119, 120, 128, 1000, 70_001];
    let seed = 7;
    for (const length of lengths) {
      const bytes = new Uint8Array(length).map(() => (seed = (seed * 1103515245 + 12345) >>> 0) >>> 24);
      expect(sha256HexSync(bytes)).toBe(await sha256Hex(bytes));
    }
    expect(sha256HexSync(new TextEncoder().encode('abc'))).toBe('ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad');
  });

  it('answers the job engine’s export asks with the bucket’s premultiplied pixels', async () => {
    const { rasters, contentA } = await setup();
    const answer = await rasters.produce({ document: 'doc', composition: 'c', layer: 'L', time: 0, source: 'svgImage', scale: 1, revision: 3, svg: request(contentA, 5, 2) });
    expect(answer).toMatchObject({ key: `${contentA}:5x2`, width: 5, height: 2, format: 'rgba8Premultiplied', origin: [0, 0], scale: 1 });
    expect(answer.data.byteLength).toBe(40);
    await expect(rasters.produce({ document: 'doc', composition: 'c', layer: 'L', time: 0, source: 'svgImage', scale: 1, revision: 3 })).rejects.toThrow(/names no SVG raster/);
  });
});
