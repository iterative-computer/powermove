import { expect, it } from 'vitest';
import { mkdtemp, readFile, rename, rm, writeFile } from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';
import { createHash } from 'node:crypto';
import { sampledFingerprint } from './media';
it('matches the desktop sampled identity across renames and samples the middle and tail', async () => {
  const dir = await mkdtemp(path.join(os.tmpdir(), 'pm-import-fingerprint-'));
  try {
    const file = path.join(dir, 'first.bin'), data = Buffer.alloc(200_003);
    for (let i = 0; i < data.length; i++) data[i] = (i * 31 + (i >>> 8)) & 255;
    await writeFile(file, data);
    const sample = 65536, hash = createHash('sha256').update(`powermove-media-v2\n${data.length}\n`);
    for (const at of [0, Math.floor((data.length - sample) / 2), data.length - sample]) hash.update(data.subarray(at, at + sample));
    const expected = `v2:${data.length}:${hash.digest('hex')}`;
    expect(await sampledFingerprint(file, data.length)).toBe(expected);
    const renamed = path.join(dir, 'renamed.mov'); await rename(file, renamed);
    expect(await sampledFingerprint(renamed, data.length)).toBe(expected);
    const changed = await readFile(renamed); changed[100_000] = changed[100_000]! ^ 255; await writeFile(renamed, changed);
    expect(await sampledFingerprint(renamed, data.length)).not.toBe(expected);
    await writeFile(file, ''); expect(await sampledFingerprint(file, 0)).toBe(`v2:0:${createHash('sha256').update('powermove-media-v2\n0\n').digest('hex')}`);
  } finally { await rm(dir, { recursive: true, force: true }); }
});
