/** Original sequence manifests, kept locally beside the host's media index.
 * A sequence's identity includes source bytes, order and exact rate, not paths.
 * Generation rechecks every frame; the page never supplies a manifest path. */
import { mkdir, readFile, realpath, rename, rm, stat, writeFile } from 'node:fs/promises';
import { randomUUID } from 'node:crypto';
import path from 'node:path';
import { blake3Hex } from '../shared/blake3';
import { resolveFrameRate } from '../shared/frame-rate';
import { MAX_SEQUENCE_FRAMES } from '../shared/animated-image';

const HASH = /^[a-f0-9]{64}$/;
type Frame = { path: string; hash: string };
type Manifest = { powermoveSequence: 1; rate: { num: number; den: number }; frames: Frame[] };

export const sequenceContentHash = (rate: Manifest['rate'], hashes: string[]): string =>
  blake3Hex(new TextEncoder().encode(`pm-sequence-v1\n${rate.num}/${rate.den}\n${hashes.map(hash => `${hash}\n`).join('')}`));

export class OriginalSequenceStore {
  constructor(private readonly root: string, private readonly hashFile: (file: string) => Promise<string>) {}

  async register(sources: string[], fps: number): Promise<string> {
    const rate = resolveFrameRate(fps);
    if (!rate || sources.length < 1 || sources.length > MAX_SEQUENCE_FRAMES) throw new Error('Invalid original image sequence.');
    const frames: Frame[] = [];
    for (const source of sources) {
      if (!path.isAbsolute(source) || source.includes('\0')) throw new Error('Choose local numbered image files.');
      const file = await realpath(source);
      const before = await stat(file);
      if (!before.isFile() || before.size < 1) throw new Error('An original sequence frame is unreadable.');
      const hash = await this.hashFile(file);
      const after = await stat(file);
      if (!HASH.test(hash) || before.size !== after.size || before.mtimeMs !== after.mtimeMs || before.ctimeMs !== after.ctimeMs) {
        throw new Error('An original sequence frame changed while importing. Try again.');
      }
      frames.push({ path: file, hash });
    }
    const exactRate = { num: rate.num, den: rate.den };
    const hash = sequenceContentHash(exactRate, frames.map(frame => frame.hash));
    const manifest: Manifest = { powermoveSequence: 1, rate: exactRate, frames };
    await mkdir(this.root, { recursive: true, mode: 0o700 });
    const temporary = path.join(this.root, `.${randomUUID()}.tmp`);
    try {
      await writeFile(temporary, JSON.stringify(manifest), { flag: 'wx', mode: 0o600 });
      await rename(temporary, path.join(this.root, `${hash}.json`));
    } finally { await rm(temporary, { force: true }); }
    return hash;
  }

  async resolve(hash: string): Promise<{ path: string; contentHash: Uint8Array } | null> {
    if (!HASH.test(hash)) return null;
    const file = path.join(this.root, `${hash}.json`);
    let manifest: Manifest;
    try {
      if ((await stat(file)).size > 32 * 1024 * 1024) return null;
      manifest = JSON.parse(await readFile(file, 'utf8'));
    } catch { return null; }
    if (!manifest || typeof manifest !== 'object' || manifest.powermoveSequence !== 1 || !Number.isSafeInteger(manifest.rate?.num) || !Number.isSafeInteger(manifest.rate?.den)
      || manifest.rate.num < 1 || manifest.rate.den < 1 || !Array.isArray(manifest.frames)
      || manifest.frames.length < 1 || manifest.frames.length > MAX_SEQUENCE_FRAMES
      || manifest.frames.some(frame => !frame || !HASH.test(frame.hash) || typeof frame.path !== 'string' || !path.isAbsolute(frame.path) || frame.path.includes('\0'))
      || sequenceContentHash(manifest.rate, manifest.frames.map(frame => frame.hash)) !== hash) return null;
    return { path: file, contentHash: Uint8Array.from(Buffer.from(hash, 'hex')) };
  }
}
