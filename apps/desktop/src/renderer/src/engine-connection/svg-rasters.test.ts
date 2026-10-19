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
import { MAX_SVG_BAND_BYTES, SvgRasterCanvas, SvgRasters, sha256Hex, type SvgAsset } from './svg-rasters';

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
    canvas: { draw } as unknown as SvgRasterCanvas
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
    expect(posted).toHaveLength(1);
    expect(posted[0]).toMatchObject({ type: 'compat', compat: 'svgPixels', key: { content: contentA, width: 6, height: 3 }, row: 0, rows: 3 });
    expect((posted[0]!['data'] as Uint8Array).byteLength).toBe(6 * 3 * 4);
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

  it('draws on the calling thread only with a hash it already has', async () => {
    const { rasters, assets, contentA } = await setup();
    expect(() => rasters.drawSync(request(contentA))).toThrow(/not hashed yet/);
    rasters.prime();
    await rasters.content(assets.get('a1')!);
    expect(rasters.drawSync(request(contentA)).byteLength).toBe(32);
  });

  it('answers the job engine’s export asks with the bucket’s premultiplied pixels', async () => {
    const { rasters, contentA } = await setup();
    const answer = await rasters.produce({ document: 'doc', composition: 'c', layer: 'L', time: 0, source: 'svgImage', scale: 1, revision: 3, svg: request(contentA, 5, 2) });
    expect(answer).toMatchObject({ key: `${contentA}:5x2`, width: 5, height: 2, format: 'rgba8Premultiplied', origin: [0, 0], scale: 1 });
    expect(answer.data.byteLength).toBe(40);
    await expect(rasters.produce({ document: 'doc', composition: 'c', layer: 'L', time: 0, source: 'svgImage', scale: 1, revision: 3 })).rejects.toThrow(/names no SVG raster/);
  });
});
