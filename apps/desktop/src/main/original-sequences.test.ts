import { mkdtemp, readFile, realpath, rename, rm, writeFile } from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';
import { expect, it } from 'vitest';
import { blake3Hex } from '../shared/blake3';
import { OriginalSequenceStore } from './original-sequences';

it('keys original sequences by ordered content and exact rate, surviving a move and relaunch', async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), 'pm-sequence-registry-'));
  try {
    const files = [path.join(root, 'frame-1.png'), path.join(root, 'frame-2.png')];
    await writeFile(files[0]!, 'first original'); await writeFile(files[1]!, 'second original');
    const hashFile = async (file: string) => blake3Hex(await readFile(file));
    const registry = new OriginalSequenceStore(path.join(root, 'manifests'), hashFile);
    const hash = await registry.register(files, 29.97);
    const source = await registry.resolve(hash);
    expect(source).not.toBeNull();
    const manifest = JSON.parse(await readFile(source!.path, 'utf8'));
    expect(manifest.rate).toEqual({ num: 30000, den: 1001 });
    expect(manifest.frames.map((frame: any) => frame.hash)).toEqual(await Promise.all(files.map(hashFile)));
    expect(await registry.register([...files].reverse(), 29.97)).not.toBe(hash);
    expect(await registry.register(files, 24)).not.toBe(hash);
    const moved = path.join(root, 'moved.png'); await rename(files[0]!, moved);
    expect(await registry.register([moved, files[1]!], 29.97)).toBe(hash);
    const relaunched = new OriginalSequenceStore(path.join(root, 'manifests'), hashFile);
    expect(JSON.parse(await readFile((await relaunched.resolve(hash))!.path, 'utf8')).frames[0].path).toBe(await realpath(moved));
    await writeFile(moved, 'changed raw frame');
    expect(await registry.register([moved, files[1]!], 29.97)).not.toBe(hash);
    expect(await relaunched.resolve('../escape')).toBeNull();
  } finally { await rm(root, { recursive: true, force: true }); }
});
