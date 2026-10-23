import { ShaderFallbackRunner } from '../engine-connection/shader-fallback';
import { SvgRasterCanvas, decodeSvg, sha256Hex, sha256HexSync } from '../engine-connection/svg-rasters';
import type { SvgRasterKey } from '../../../shared/engine-protocol';
const runner = new ShaderFallbackRunner();
const bytes = (base64: string): Uint8Array => Uint8Array.from(atob(base64), c => c.charCodeAt(0));
function base64(data: Uint8Array): string {
  let text = '';
  for (let start = 0; start < data.length; start += 8192) text += String.fromCharCode(...data.subarray(start, start + 8192));
  return btoa(text);
}
export async function runShader(request: any, inputs: string[]) {
  try {
    const textures = inputs.map(input => new Uint16Array(bytes(input).buffer));
    const rendered = await runner.render(request, textures);
    return { ok: true as const, pixels: base64(new Uint8Array(rendered.rgba16f.buffer, rendered.rgba16f.byteOffset, rendered.rgba16f.byteLength)) };
  } catch (error) { return { ok: false as const, reason: String((error as any).reason ?? 'unavailable'), message: String((error as Error).message ?? error) }; }
}

/** One level 1 SVG raster (decision S1) for a renderer without a document
 *  window (`pm-cli` through `powermove render`'s service): the file's bytes
 *  decoded as today's import decodes them and drawn by the app's own module,
 *  today's `rasterSvgAsset` and `texFor` upload read back (premultiplied
 *  RGBA8, top-down), exactly as the hidden host's tab draws them. */
const svgCanvas = new SvgRasterCanvas();
export async function runSvgRaster(key: SvgRasterKey, svg: string) {
  try {
    const data = bytes(svg);
    // As `SvgRasters` answers `stale`: never draw other bytes for the key.
    // `crypto.subtle` needs a secure context; the CLI's page may not be one.
    const hash = globalThis.crypto?.subtle ? await sha256Hex(data) : sha256HexSync(data);
    if (hash !== key.content) return { ok: false as const, reason: 'stale', message: 'The SVG file changed after the render planned it.' };
    const image = await decodeSvg(data);
    const pixels = key.texture ? svgCanvas.drawTexture(image, key.width, key.height) : svgCanvas.draw(image, key.width, key.height);
    return { ok: true as const, pixels: base64(pixels) };
  } catch (error) { return { ok: false as const, reason: String((error as any).reason ?? 'unavailable'), message: String((error as Error).message ?? error) }; }
}
// The standalone Node CLI injects this same module into its minimal ANGLE page.
(globalThis as any).runNativeShader = runShader;
(globalThis as any).runNativeSvgRaster = runSvgRaster;
