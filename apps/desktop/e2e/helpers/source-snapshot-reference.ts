/** Independent source-media oracle; disposable synthetic files only. */
import { execFile } from 'node:child_process';
import { createHash } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import { promisify } from 'node:util';
import { readSdrVideoTags, referenceSrgbCodes, uniformPlanarYuv, type ColourTriple } from './source-colour-reference';
import { decodeCapturedPng, sourceFfmpeg } from './source-media';

const run = promisify(execFile);

export async function decodedSourceReference(file: string, times: number[]) {
  const { stdout } = await run(sourceFfmpeg().replace(/ffmpeg$/, 'ffprobe'), ['-v', 'error', '-select_streams', 'v:0',
    '-show_entries', 'stream=width,height,pix_fmt,color_range,color_space,color_transfer,color_primaries', '-of', 'json', file], { timeout: 30_000 });
  const stream = JSON.parse(stdout).streams?.[0];
  if (!stream) throw new Error('Source reference has no video stream');
  const tags = readSdrVideoTags(stream);
  const frames = [];
  for (const time of times) {
    // No scale, RGB conversion, or forced pixel format: retain the decoded
    // media's actual planar values, including a rendition's quantization.
    const { stdout } = await run(sourceFfmpeg(), ['-v', 'error', '-nostdin', '-ss', String(time), '-i', file,
      '-map', '0:v:0', '-frames:v', '1', '-f', 'rawvideo', 'pipe:1'],
    { timeout: 30_000, encoding: 'buffer', maxBuffer: 16 * 1024 * 1024 });
    const { yuv, bits } = uniformPlanarYuv(stdout, stream.width, stream.height, stream.pix_fmt);
    frames.push({ time, yuv, bits, codes: referenceSrgbCodes(yuv, tags, bits) });
  }
  return { sha256: createHash('sha256').update(await readFile(file)).digest('hex'), stream, tags, frames };
}

/** Every channel of every pixel, including opacity, for solid decoded media.
 * Screenshots may name an interior rectangle to exclude rounded corners and
 * asset chrome; their layout is separately compared with legacy geometry.
 */
export async function solidCaptureError(file: string, expected: ColourTriple, interior = false) {
  const { size, pixels } = await decodeCapturedPng(file);
  const [width, height] = size as [number, number];
  // The source card's duration badge occupies its top-left corner. The
  // central 60% × 40% is clear of that badge and the rounded outer edge.
  const insetX = interior ? Math.ceil(width * .2) : 0, insetY = interior ? Math.ceil(height * .3) : 0;
  let max = 0, sum = 0, samples = 0, alphaMax = 0;
  for (let y = insetY; y < height - insetY; y++) for (let x = insetX; x < width - insetX; x++) {
    const at = 4 * (y * width + x);
    for (let channel = 0; channel < 3; channel++) {
      const error = Math.abs(pixels[at + channel]! - expected[channel]!);
      max = Math.max(max, error); sum += error; samples++;
    }
    alphaMax = Math.max(alphaMax, Math.abs(pixels[at + 3]! - 255));
  }
  if (!samples || pixels.length !== width * height * 4) throw new Error('Empty or malformed reference capture');
  return { max, mean: sum / samples, alphaMax, pixels: samples / 3, size, expected };
}
