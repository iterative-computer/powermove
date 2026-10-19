import { mkdtemp, readFile, readdir, rm, stat, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { afterEach, expect, it } from 'vitest';
import { packProjectFileBlob } from '../../renderer/src/legacy/core/project-file';
import { decodeProjectContainer } from '../../shared/project-container';
import { packAgentProject } from './project-packer';

const roots: string[] = [];
afterEach(async () => { for (const root of roots.splice(0)) await rm(root, { recursive: true, force: true }); });
const tempRoot = async (name: string) => { const root = await mkdtemp(path.join(os.tmpdir(), name)); roots.push(root); return root; };

/** A project with every kind of asset today's packer meets: stored media, a
 *  linked original, media reachable only by Undo, an asset whose bytes are
 *  gone, and an offline linked original. */
async function fixture(root: string) {
  const files = { stored: path.join(root, 'stored.bin'), linked: path.join(root, 'linked.mov'), undo: path.join(root, 'deleted.png') };
  await writeFile(files.stored, new Uint8Array([1, 2, 3, 4]));
  await writeFile(files.linked, new Uint8Array([5, 6, 7]));
  await writeFile(files.undo, new Uint8Array([8, 9]));
  const project = { id: 'p', compId: 'c', name: 'Launch', layers: [], w: 320, h: 180, assets: {
    stored: { id: 'stored', name: 'Stored', type: 'video/mp4' },
    linked: { id: 'linked', name: 'Linked', type: 'video/quicktime', storage: 'linked', link: { path: files.linked, hash: 'a'.repeat(64), size: 3 } },
    gone: { id: 'gone', name: 'Gone', type: 'image/png' },
    offline: { id: 'offline', name: 'Offline', type: 'audio/wav', storage: 'linked', link: { path: path.join(root, 'unmounted.wav'), hash: 'b'.repeat(64), size: 9 } }
  } };
  const document = { v: '1.1.3', proj: project, ws: { name: 'Editing' }, history: { entries: [{ forward: [], backward: [{ path: ['assets', 'deleted'], exists: true, value: { id: 'deleted', name: 'Deleted', type: 'image/png' } }] }] } };
  const paths: Record<string, string> = { stored: files.stored, linked: files.linked, deleted: files.undo };
  return { project, document, files, paths };
}

it('writes today\'s agent save byte for byte, skipping media whose bytes are gone instead of refusing', async () => {
  const root = await tempRoot('pm-agent-pack-');
  const { project, document, paths } = await fixture(root);
  const directory = path.join(root, 'exports');
  const output = await packAgentProject(project, { directory, resolveAsset: async id => paths[id] ? { path: paths[id]! } : null }, document);
  // Today's packer, with the media store answering the same bytes (and nothing for the missing ones).
  const store = {
    get: async (asset: any) => paths[asset.id] ? new Blob([await readFile(paths[asset.id]!)], { type: asset.type }) : null,
    put: async () => true
  };
  const today = new Uint8Array(await (await packProjectFileBlob(structuredClone(document), store)).arrayBuffer());
  const written = new Uint8Array(await readFile(output));
  expect(Buffer.from(written).subarray(0, 5).toString()).toBe('PMV3\n');
  expect(Buffer.compare(written, today)).toBe(0);
  const restored = decodeProjectContainer(written) as { document: any; media: Array<{ id: string; data: Uint8Array }> };
  // The missing asset and the offline original stay in the document, editable on reopening.
  expect(restored.document.proj.assets.gone).toEqual(project.assets.gone);
  expect(restored.document.proj.assets.offline).toEqual(project.assets.offline);
  expect(restored.document.proj.assets.linked.storage).toBeUndefined();
  expect(restored.document.history).toEqual(document.history);
  expect((await stat(output)).mode & 0o777).toBe(0o600);
  expect(await readdir(directory)).toEqual(['Launch.pmv']);
});

it('saves when no media can be found at all, as today', async () => {
  const root = await tempRoot('pm-agent-pack-offline-');
  const { project, document } = await fixture(root);
  const output = await packAgentProject(project, { directory: path.join(root, 'exports'), resolveAsset: async () => { throw new Error('store offline'); } }, document);
  const restored = decodeProjectContainer(new Uint8Array(await readFile(output))) as { document: any; media: unknown[] };
  expect(restored.media).toEqual([]);
  expect(restored.document.proj.assets.linked.storage).toBe('linked');
});

it('reserves exclusive names for concurrent saves', async () => {
  const root = await tempRoot('pm-agent-pack-names-');
  const { project, paths } = await fixture(root);
  const directory = path.join(root, 'exports');
  const options = { directory, resolveAsset: async (id: string) => paths[id] ? { path: paths[id]! } : null };
  const outputs = await Promise.all([packAgentProject(project, options), packAgentProject(project, options)]);
  expect(new Set(outputs).size).toBe(2);
  expect((await readdir(directory)).sort()).toEqual(['Launch (2).pmv', 'Launch.pmv']);
});

it('fails with a named error and leaves no file when an original changes while it is copied', async () => {
  const root = await tempRoot('pm-agent-pack-changed-');
  const { project, document, files, paths } = await fixture(root);
  const directory = path.join(root, 'exports');
  // The stored original changes after it was pinned (while the next asset resolves).
  const resolveAsset = async (id: string) => {
    if (id === 'linked') await writeFile(files.stored, new Uint8Array([1, 2, 3, 4, 5, 6]));
    return paths[id] ? { path: paths[id]! } : null;
  };
  await expect(packAgentProject(project, { directory, resolveAsset }, document)).rejects.toThrow(/^mediaChanged: Original media stored changed/);
  expect(await readdir(directory)).toEqual([]);
});
