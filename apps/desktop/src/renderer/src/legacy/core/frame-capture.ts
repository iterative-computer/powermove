import type { PMRegistry } from '../registry';

let straightAlphaTable: Uint8Array | undefined;

/** Preserve the established integer unpremultiplication exactly. A 64 KiB
 * table replaces three divisions per pixel, including transparent black. */
export function writeStraightAlphaPixels(pixels: Uint8Array, width: number, height: number, output: Uint8ClampedArray): void {
  if (!straightAlphaTable) {
    straightAlphaTable = new Uint8Array(256 * 256);
    for (let alpha = 1; alpha < 256; alpha++) for (let color = 0; color < 256; color++) {
      straightAlphaTable[alpha * 256 + color] = Math.min(255, (color * 255 / alpha) | 0);
    }
  }
  const table = straightAlphaTable;
  for (let y = 0; y < height; y++) {
    let source = (height - 1 - y) * width * 4, target = y * width * 4;
    for (let x = 0; x < width; x++, source += 4, target += 4) {
      const alpha = pixels[source + 3]!;
      if (alpha === 255) {
        output[target] = pixels[source]!;
        output[target + 1] = pixels[source + 1]!;
        output[target + 2] = pixels[source + 2]!;
      } else if (alpha === 0) {
        output[target] = output[target + 1] = output[target + 2] = 0;
      } else {
        const offset = alpha * 256;
        output[target] = table[offset + pixels[source]!]!;
        output[target + 1] = table[offset + pixels[source + 1]!]!;
        output[target + 2] = table[offset + pixels[source + 2]!]!;
      }
      output[target + 3] = alpha;
    }
  }
}

/** A context can be lost during readback, before its event handler clears GL.gl.
 * Do not deliver the resulting empty/invalid frame to an encoder. */
export function captureFramePixels(PM: PMRegistry, time: number, width: number, height: number,
  options: Record<string, unknown>, capture?: (options: Record<string, unknown>) => Uint8Array<ArrayBuffer> | null): Uint8Array<ArrayBuffer> {
  const pixels = capture ? capture(options) : PM.GL.renderToPixels(time, width, height, options);
  if (!pixels || pixels.length !== width * height * 4 || PM.GL.contextLost || PM.GL.gl?.isContextLost?.()) {
    throw new Error('The GPU could not capture the export frame');
  }
  return pixels;
}

/** Capture top-down opaque RGBA8, with transparency displayed over black.
 * The GPU capture pass converts orientation and alpha before readback, so the
 * ImageData shares that owned buffer without another allocation or CPU pass. */
export function renderOpaqueFrame(PM: PMRegistry, time: number, width: number, height: number,
  options: { mblur?: boolean; mbSamples?: number } = {}): ImageData {
  const quality = PM.quality;
  try {
    PM.quality = 1;
    const motionBlur = options.mblur !== false;
    const pixels = captureFramePixels(PM, time, width, height, {
      mblur: motionBlur, mbSamples: motionBlur ? Math.max(1, Number(options.mbSamples) || 16) : 1,
      shutter: PM.proj.shutter || .5, opaque: true, topDownOpaque: true,
    });
    return new ImageData(new Uint8ClampedArray(pixels.buffer, pixels.byteOffset, pixels.byteLength), width, height);
  } finally {
    PM.quality = quality;
  }
}
