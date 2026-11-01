import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { expect, it } from 'vitest';
import { proxyReferences } from './references';
import { encodeStorageJSON } from '../storage-codec';
it('finds supplemental proxy bindings in compressed and unknown nested logical state', async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), 'pm-tr-references-'));
  try {
    const source = { takes: [{ assets: { a: { storageKey: 'media:one', playbackProxyVersion: 1, fingerprint: 'v2:3:synthetic', sourcePath: '/synthetic/a.mov' } } }], padding: ' '.repeat(40_000), unknown: { id: 'media:opaque', playbackProxy: true } };
    await writeFile(path.join(root, 'takes.json'), await encodeStorageJSON(JSON.stringify(source)));
    expect(await proxyReferences(root)).toEqual({ 'media:one': [{ fingerprint: 'v2:3:synthetic', sourcePath: '/synthetic/a.mov' }], 'media:opaque': [{ fingerprint: null, sourcePath: null }] });
    await writeFile(path.join(root, 'broken.json'), '{broken');
    await expect(proxyReferences(root)).rejects.toThrow();
  } finally { await rm(root, { recursive: true, force: true }); }
});
