import { mkdtemp, readFile, readdir, rm, stat, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { expect, it } from 'vitest';
import { decodeIncrementalProject } from '../../shared/project-incremental';
import { packAgentProject } from './project-packer';

it('packs original and linked media into exclusive owner-only self-contained saves', async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), 'pm-agent-pack-'));
  try {
    const original = path.join(root, 'original.bin');
    await writeFile(original, new Uint8Array([1, 2, 3, 4]));
    const project = { id: 'p', compId: 'c', name: 'Launch', layers: [], w: 320, h: 180, assets: {
      a: { id: 'a', name: 'Original', type: 'video/test', storage: 'linked', link: { path: original, hash: 'a'.repeat(64), size: 4 } }
    } };
    const directory = path.join(root, 'exports');
    const options = { directory, resolveAsset: async () => ({ path: original }) };
    const outputs = await Promise.all([packAgentProject(project, options), packAgentProject(project, options)]);
    expect(new Set(outputs).size).toBe(2);
    await rm(original);
    for (const output of outputs) {
      const restored = decodeIncrementalProject(await readFile(output));
      expect(restored.document).toEqual({ ...project, assets: { a: { id: 'a', name: 'Original', type: 'video/test', contentHash: 'a'.repeat(64), sourcePath: original } } });
      expect(restored.media.map(m => [m.id, [...m.data]])).toEqual([['a', [1, 2, 3, 4]]]);
      expect((await stat(output)).mode & 0o777).toBe(0o600);
    }
    expect((await readdir(directory)).sort()).toEqual(['Launch (2).pmv', 'Launch.pmv']);
    await expect(packAgentProject(project, { directory, resolveAsset: async () => null })).rejects.toThrow('mediaOffline');
    expect(await readdir(directory)).toHaveLength(2);
  } finally { await rm(root, { recursive: true, force: true }); }
});

it('preserves the envelope and embeds originals reachable only by Undo', async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), 'pm-agent-history-pack-'));
  try {
    const original = path.join(root, 'deleted.png');
    await writeFile(original, new Uint8Array([8, 9]));
    const project = { id: 'p', compId: 'c', name: 'History', layers: [], w: 320, h: 180, assets: {} };
    const document = { v: '1.1.3', proj: project, ws: { name: 'Editing' }, history: { entries: [{ forward: [], backward: [{ path: ['assets', 'deleted'], exists: true, value: { id: 'deleted', type: 'image/png' } }] }] } };
    const output = await packAgentProject(project, { directory: path.join(root, 'exports'), resolveAsset: async id => id === 'deleted' ? { path: original } : null }, document);
    const restored = decodeIncrementalProject(await readFile(output));
    expect(restored.document).toEqual(document);
    expect(restored.media.map(m => [m.id, [...m.data]])).toEqual([['deleted', [8, 9]]]);
  } finally { await rm(root, { recursive: true, force: true }); }
});
