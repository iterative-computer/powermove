import { createHash } from 'node:crypto';
import { beforeEach, expect, it, vi } from 'vitest';

const raster = vi.hoisted(() => ({ draw: vi.fn(), drawTexture: vi.fn(), decode: vi.fn() }));
vi.mock('../engine-connection/shader-fallback', () => ({ ShaderFallbackRunner: class {} }));
vi.mock('../engine-connection/svg-rasters', async importOriginal => {
  const actual = await importOriginal<typeof import('../engine-connection/svg-rasters')>();
  return { ...actual, decodeSvg: raster.decode, SvgRasterCanvas: class { draw = raster.draw; drawTexture = raster.drawTexture; } };
});
import { runSvgRaster } from './native-hidden';

const svg = Buffer.from('<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"/>');
const key = { content: createHash('sha256').update(svg).digest('hex'), width: 1, height: 1 };
beforeEach(() => {
  vi.clearAllMocks();
  raster.decode.mockResolvedValue('decoded image');
  raster.draw.mockReturnValue(Uint8Array.of(32, 0, 0, 128));
  raster.drawTexture.mockReturnValue(Uint8Array.of(64, 0, 0, 128));
});

it('draws a content-verified canvas bucket with the app module', async () => {
  expect(await runSvgRaster(key, svg.toString('base64'))).toEqual({ ok: true, pixels: Buffer.from([32, 0, 0, 128]).toString('base64') });
  expect(raster.decode).toHaveBeenCalledWith(new Uint8Array(svg));
  expect(raster.draw).toHaveBeenCalledWith('decoded image', 1, 1);
  expect(raster.drawTexture).not.toHaveBeenCalled();
});

it('preserves the straight-alpha material-map upload from the merged protocol', async () => {
  expect(await runSvgRaster({ ...key, texture: true }, svg.toString('base64'))).toEqual({ ok: true, pixels: Buffer.from([64, 0, 0, 128]).toString('base64') });
  expect(raster.drawTexture).toHaveBeenCalledWith('decoded image', 1, 1);
  expect(raster.draw).not.toHaveBeenCalled();
});

it('refuses changed bytes before decoding or drawing them under the planned hash', async () => {
  expect(await runSvgRaster(key, Buffer.from('<svg/>').toString('base64'))).toMatchObject({ ok: false, reason: 'stale' });
  expect(raster.decode).not.toHaveBeenCalled();
  expect(raster.draw).not.toHaveBeenCalled();
  expect(raster.drawTexture).not.toHaveBeenCalled();
});
