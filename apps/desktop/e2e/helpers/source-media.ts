/** Public-fixture source checks; all files live in the caller's disposable/evidence directory. */
import { createHash } from 'node:crypto';
import { execFile } from 'node:child_process';
import { readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { promisify } from 'node:util';
import { fixturePath } from './media';
import { repoRoot } from './app';

const exec = promisify(execFile);
export const sourceFfmpeg = () => process.env.PM_FFMPEG_DIR ? path.join(process.env.PM_FFMPEG_DIR, 'bin/ffmpeg') : 'ffmpeg';

export async function renditionAudioFixture(directory: string): Promise<string> {
  const file = path.join(directory, 'rendition-audio.mov');
  await exec(sourceFfmpeg(), ['-hide_banner', '-loglevel', 'error', '-nostdin',
    '-i', fixturePath('prores422.mov'), '-i', fixturePath('tone.wav'),
    // Picture outlasts the soundtrack, as browser source playback must do.
    '-map', '0:v:0', '-map', '1:a:0', '-c:v', 'copy', '-c:a', 'pcm_s16le', '-y', file],
  { timeout: 60_000 });
  return file;
}

export async function decodedRenditionIdentity(directory: string, name: string, kind: 'image' | 'video', reference: number[], actual: number[]) {
  const extension = kind === 'image' ? 'png' : 'webm';
  const expectedFile = path.join(directory, `${name}-reference.${extension}`);
  const actualFile = path.join(directory, `${name}-native.${extension}`);
  await writeFile(expectedFile, new Uint8Array(reference));
  await writeFile(actualFile, new Uint8Array(actual));
  const decode = async (file: string, audio: boolean): Promise<Buffer> => {
    const output = await exec(sourceFfmpeg(), ['-hide_banner', '-loglevel', 'error', '-nostdin', '-threads', '2', '-i', file,
      '-map', audio ? '0:a:0' : '0:v:0', ...(audio ? ['-ac', '2', '-ar', '48000', '-f', 'f32le'] : ['-pix_fmt', 'rgba', '-f', 'rawvideo']), 'pipe:1'],
    { timeout: 60_000, encoding: 'buffer', maxBuffer: 128 * 1024 * 1024 });
    if (!output.stdout.length) throw new Error(`Empty decoded ${audio ? 'audio' : 'picture'} from ${name}`);
    return output.stdout;
  };
  const digest = (bytes: Uint8Array) => createHash('sha256').update(bytes).digest('hex');
  const pictureReference = await decode(expectedFile, false), pictureActual = await decode(actualFile, false);
  const picture = { equal: pictureReference.equals(pictureActual), expectedBytes: pictureReference.length, actualBytes: pictureActual.length,
    expectedHash: digest(pictureReference), actualHash: digest(pictureActual) };
  let audio: typeof picture | null = null;
  if (kind === 'video') {
    const expected = await decode(expectedFile, true), received = await decode(actualFile, true);
    audio = { equal: expected.equals(received), expectedBytes: expected.length, actualBytes: received.length,
      expectedHash: digest(expected), actualHash: digest(received) };
  }
  return { byteEqual: Buffer.from(reference).equals(Buffer.from(actual)), picture, audio };
}

/** Same −85 dBFS whole-mix ceiling as pm-audio/tests/legacy_parity.rs; no masks or alignment. */
export async function sourcePcmComparison(directory: string, name: string, input: number[], expected: number[]) {
  const source = path.join(directory, name), output = path.join(directory, `${name}.native.f32le`);
  await writeFile(source, new Uint8Array(input));
  const reference = new Float32Array(expected);
  await writeFile(path.join(directory, `${name}.reference.f32le`), new Uint8Array(reference.buffer));
  await exec(path.resolve(repoRoot, '../../engine/target/release/examples/source_monitor_pcm'), [source, output, '48000'], { timeout: 60_000 });
  const file = await readFile(output);
  const native = new Float32Array(new Uint8Array(file).buffer);
  let square = 0, signal = 0, max = 0;
  const length = Math.max(reference.length, native.length);
  for (let i = 0; i < length; i++) {
    const a = reference[i] ?? 0, b = native[i] ?? 0, difference = a - b;
    square += difference * difference; signal += a * a; max = Math.max(max, Math.abs(difference));
  }
  return { expectedFrames: reference.length / 2, nativeFrames: native.length / 2, sameFrames: reference.length === native.length,
    channels: 2, sampleRate: 48000, alignmentFrames: 0, peakResidual: max,
    rmsResidualDbfs: 20 * Math.log10(Math.max(1e-15, Math.sqrt(square / length))), signalRms: Math.sqrt(signal / length), ceilingDbfs: -85 };
}
