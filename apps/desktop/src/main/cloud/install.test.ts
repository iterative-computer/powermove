import { createHash } from 'node:crypto';
import * as fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { afterEach, describe, expect, it, vi } from 'vitest';
import { snapshot, type SnapshotInput } from '@powermove/registry/snapshot';
import { writeTarGz } from '@powermove/registry/tar';
import { ApiError, type MeDto } from '@powermove/registry/wire';

import type { ExtensionRecord } from '../../shared/extensions';
import { createStoreInstaller, StoreLocalError, type StoreInstallerRegistry } from './install';
import { createProvenanceStore } from './provenance';
import type { ReleaseByIdResult, StoreClient, VersionsItem } from './store-client';

const REPO = '11111111-1111-4111-8111-111111111111';
const OWNER = '22222222-2222-4222-8222-222222222222';
const R1 = '33333333-3333-4333-8333-333333333333';
const R2 = '44444444-4444-4444-8444-444444444444';

const roots: string[] = [];
afterEach(async () => {
  await Promise.all(roots.splice(0).map((dir) => fs.rm(dir, { recursive: true, force: true })));
});

const text = (value: string): Uint8Array => new TextEncoder().encode(value);
const sha256 = (bytes: Uint8Array): string => createHash('sha256').update(bytes).digest('hex');

function tree(version: string, id = 'glass-blur', body = 'export default {}\n'): SnapshotInput[] {
  return [
    { path: 'manifest.json', bytes: text(`${JSON.stringify({ id, name: 'Glass blur', version, apiVersion: 2, contributes: ['effects'] }, null, 2)}\n`) },
    { path: 'index.ts', bytes: text(body) },
    { path: 'shaders/glass.frag', bytes: text(`// ${version}\nvoid main() {}\n`) }
  ];
}

interface Built { release: ReleaseByIdResult; tar: Uint8Array; files: SnapshotInput[] }

async function build(releaseId: string, version: string, files = tree(version)): Promise<Built> {
  const snap = await snapshot(files);
  const tar = await writeTarGz(files);
  const manifest = JSON.parse(new TextDecoder().decode(files[0]!.bytes)) as { id: string; name: string };
  return {
    files,
    tar,
    release: {
      id: releaseId, repoId: REPO, version, commitSha: 'c'.repeat(40), treeSha: snap.treeSha, tarSha256: sha256(tar),
      apiVersion: 2, fileCount: files.length, sizeBytes: tar.length, notes: null, publishedAt: '2026-09-20T00:00:00.000Z',
      yankedAt: null, basedOnReleaseId: null, handle: 'mara', slug: 'glass-blur',
      manifest: { id: manifest.id, name: manifest.name, version, apiVersion: 2, contributes: ['effects'], vars: [], forkedFrom: null, description: null, permissions: [] }
    }
  };
}

function fakeClient(releases: Built[], latest: () => Built) {
  const byId = (id: string): Built => {
    const found = releases.find((entry) => entry.release.id === id);
    if (!found) throw new ApiError({ error: 'not_found' });
    return found;
  };
  const unused = async (): Promise<never> => { throw new Error('not used'); };
  const addInstall = vi.fn(async (_repoId: string, _releaseId: string) => undefined);
  const deleteInstall = vi.fn(async (_repoId: string) => undefined);
  const client: StoreClient = {
    browse: unused, extensions: unused, detail: unused, tree: unused, file: unused, compare: unused, listInstalls: unused,
    release: async (id) => structuredClone(byId(id).release),
    tar: async (id) => byId(id).tar.slice(),
    versions: async (items) => ({
      items: items.map((item): VersionsItem => {
        const head = latest().release;
        return {
          repoId: item.repoId, state: 'ok', ownerPublisherId: OWNER, current: { yanked: false },
          latest: { releaseId: head.id, version: head.version, treeSha: head.treeSha, tarSha256: head.tarSha256, apiVersion: 2 },
          handle: 'mara', slug: 'glass-blur'
        };
      })
    }),
    addInstall,
    deleteInstall
  };
  return Object.assign(client, { addInstall, deleteInstall });
}

const me: MeDto = {
  user: { id: '55555555-5555-4555-8555-555555555555', name: 'Jude', email: 'jude@example.com', image: null },
  publisher: { id: '66666666-6666-4666-8666-666666666666', handle: 'jude', tombstoned: false },
  settings: { rememberInstalls: true }
};

async function setup(options: { builtins?: string[]; rename?: (from: string, to: string) => Promise<void> } = {}) {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'powermove-install-'));
  roots.push(root);
  const userDir = path.join(root, 'extensions');
  await fs.mkdir(userDir, { recursive: true });
  const v1 = await build(R1, '1.0.0');
  const v2 = await build(R2, '1.1.0', tree('1.1.0', 'glass-blur', 'export default { v: 2 }\n'));
  let head = v1;
  const client = fakeClient([v1, v2], () => head);
  const registry: StoreInstallerRegistry = {
    userDir,
    list: (): ExtensionRecord[] => (options.builtins ?? []).map((id) => ({
      id, scope: 'builtin', manifest: null, dir: `/builtin/${id}`, enabled: true, bundleUrl: null, bundleHash: null, health: { state: 'ok' }, updatedAt: 0
    })),
    refresh: async () => undefined,
    emitChanged: () => undefined
  };
  const provenance = createProvenanceStore(root);
  const notify = vi.fn();
  const installer = createStoreInstaller({
    registry,
    provenance,
    store: client,
    me: () => me,
    signedIn: () => true,
    notifyUpdates: notify,
    log: () => undefined,
    now: () => 1234,
    ...(options.rename ? { rename: options.rename } : {})
  });
  return { root, userDir, installer, provenance, client, registry, notify, v1, v2, setHead: (next: Built) => { head = next; } };
}

async function readFolder(dir: string): Promise<Record<string, string>> {
  const out: Record<string, string> = {};
  async function walk(current: string, prefix: string): Promise<void> {
    for (const entry of await fs.readdir(current, { withFileTypes: true })) {
      const full = path.join(current, entry.name);
      const rel = prefix ? `${prefix}/${entry.name}` : entry.name;
      if (entry.isDirectory()) await walk(full, rel);
      else out[rel] = await fs.readFile(full, 'utf8');
    }
  }
  await walk(dir, '');
  return out;
}

const expected = (files: SnapshotInput[]): Record<string, string> =>
  Object.fromEntries(files.map((file) => [file.path, new TextDecoder().decode(file.bytes)]));

async function exists(target: string): Promise<boolean> {
  return fs.lstat(target).then(() => true, () => false);
}

describe('store installer', () => {
  it('installs a release byte for byte and records where it came from', async () => {
    const { userDir, installer, provenance, client, v1 } = await setup();
    const result = await installer.installRelease({ repoId: REPO, releaseId: R1 });
    expect(result).toEqual({ localId: 'glass-blur', needsSetup: false });
    expect(await readFolder(path.join(userDir, 'glass-blur'))).toEqual(expected(v1.files));
    expect(await provenance.get('glass-blur')).toEqual({
      localId: 'glass-blur',
      envKey: REPO,
      origin: {
        repoId: REPO, releaseId: R1, coordinate: 'mara/glass-blur', version: '1.0.0',
        treeSha: v1.release.treeSha, commitSha: 'c'.repeat(40), ownerPublisherId: OWNER
      },
      upstream: { releaseId: R1, treeSha: v1.release.treeSha }
    });
    expect(client.addInstall).toHaveBeenCalledWith(REPO, R1);
    expect(await fs.readdir(path.join(userDir, '.staging'))).toEqual([]);
    expect(await installer.isModified('glass-blur')).toBe(false);
  });

  it('aborts on a SHA-256 mismatch and leaves no folder behind', async () => {
    const { userDir, installer, provenance, v1 } = await setup();
    v1.release.tarSha256 = 'f'.repeat(64);
    const error = await installer.installRelease({ repoId: REPO, releaseId: R1 }).catch((caught: unknown) => caught);
    expect(error).toBeInstanceOf(StoreLocalError);
    expect((error as StoreLocalError).code).toBe('integrity');
    expect(await exists(path.join(userDir, 'glass-blur'))).toBe(false);
    expect(await provenance.get('glass-blur')).toBeNull();
  });

  it('aborts when the tree hash does not match the release', async () => {
    const { userDir, installer, v1 } = await setup();
    v1.release.treeSha = 'a'.repeat(40);
    await expect(installer.installRelease({ repoId: REPO, releaseId: R1 })).rejects.toMatchObject({ code: 'integrity' });
    expect(await exists(path.join(userDir, 'glass-blur'))).toBe(false);
    expect(await exists(path.join(userDir, '.staging', 'glass-blur'))).toBe(false);
  });

  it('refuses an id that is already a folder here, and names it', async () => {
    const { userDir, installer } = await setup();
    await fs.mkdir(path.join(userDir, 'glass-blur'));
    await fs.writeFile(path.join(userDir, 'glass-blur', 'mine.txt'), 'keep');
    const error = await installer.installRelease({ repoId: REPO, releaseId: R1 }).catch((caught: unknown) => caught);
    expect(error).toBeInstanceOf(ApiError);
    expect((error as ApiError).body).toMatchObject({ error: 'id_collision' });
    expect((error as ApiError).body.detail).toContain('glass-blur');
    expect(await readFolder(path.join(userDir, 'glass-blur'))).toEqual({ 'mine.txt': 'keep' });
  });

  it('refuses an id that belongs to a built-in', async () => {
    const { userDir, installer } = await setup({ builtins: ['glass-blur'] });
    await expect(installer.installRelease({ repoId: REPO, releaseId: R1 })).rejects.toMatchObject({ code: 'builtin_collision' });
    expect(await exists(path.join(userDir, 'glass-blur'))).toBe(false);
  });

  it('swaps in an unmodified update and moves origin and upstream', async () => {
    const { userDir, installer, provenance, v2, setHead } = await setup();
    await installer.installRelease({ repoId: REPO, releaseId: R1 });
    setHead(v2);
    await expect(installer.updateRelease('glass-blur')).resolves.toEqual({ kind: 'updated', localId: 'glass-blur', version: '1.1.0' });
    expect(await readFolder(path.join(userDir, 'glass-blur'))).toEqual(expected(v2.files));
    const record = await provenance.get('glass-blur');
    expect(record?.origin).toMatchObject({ releaseId: R2, version: '1.1.0', treeSha: v2.release.treeSha });
    expect(record?.upstream).toEqual({ releaseId: R2, treeSha: v2.release.treeSha });
    expect(await fs.readdir(path.join(userDir, '.trash'))).toEqual([]);
  });

  it('puts the old folder back when the second rename fails', async () => {
    let failSwap = false;
    const { userDir, installer, provenance, v1, v2, setHead } = await setup({
      rename: async (from, to) => {
        if (failSwap && from.includes(`${path.sep}.staging${path.sep}`) && to.endsWith(`${path.sep}glass-blur`)) throw new Error('EXDEV');
        await fs.rename(from, to);
      }
    });
    await installer.installRelease({ repoId: REPO, releaseId: R1 });
    setHead(v2);
    failSwap = true;
    await expect(installer.updateRelease('glass-blur')).rejects.toMatchObject({ code: 'local' });
    expect(await readFolder(path.join(userDir, 'glass-blur'))).toEqual(expected(v1.files));
    expect((await provenance.get('glass-blur'))?.origin?.releaseId).toBe(R1);
    expect(await fs.readdir(path.join(userDir, '.staging'))).toEqual([]);
    expect(await fs.readdir(path.join(userDir, '.trash'))).toEqual([]);
  });

  it('stages a modified update beside the folder and leaves the folder alone', async () => {
    const { userDir, installer, provenance, v2, setHead } = await setup();
    await installer.installRelease({ repoId: REPO, releaseId: R1 });
    const folder = path.join(userDir, 'glass-blur');
    await fs.writeFile(path.join(folder, 'index.ts'), 'export default { mine: true }\n');
    // Dot-entries are outside the tree hash: a fork note never counts as a change.
    await fs.writeFile(path.join(folder, '.forked-from'), 'x');
    expect(await installer.isModified('glass-blur')).toBe(true);
    const before = await readFolder(folder);
    setHead(v2);
    const result = await installer.updateRelease('glass-blur');
    const target = path.join(userDir, '.updates', 'glass-blur');
    expect(result).toEqual({ kind: 'staged-for-merge', localId: 'glass-blur', version: '1.1.0', path: target });
    expect(await readFolder(folder)).toEqual(before);
    expect(await readFolder(target)).toEqual(expected(v2.files));
    const record = await provenance.get('glass-blur');
    expect(record?.pendingUpdate).toEqual({ releaseId: R2, version: '1.1.0', path: target });
    expect(record?.origin?.releaseId).toBe(R1);
  });

  it('does not count a dot-entry as a change', async () => {
    const { userDir, installer } = await setup();
    await installer.installRelease({ repoId: REPO, releaseId: R1 });
    await fs.writeFile(path.join(userDir, 'glass-blur', '.forked-from'), 'x');
    expect(await installer.isModified('glass-blur')).toBe(false);
  });

  it('uninstall removes through the given path and clears provenance', async () => {
    const { userDir, installer, provenance, client } = await setup();
    await installer.installRelease({ repoId: REPO, releaseId: R1 });
    const remove = vi.fn(async (id: string) => {
      await fs.rm(path.join(userDir, id), { recursive: true });
      return true;
    });
    await expect(installer.uninstall('glass-blur', remove)).resolves.toEqual({ removed: true });
    expect(remove).toHaveBeenCalledWith('glass-blur');
    expect(await provenance.get('glass-blur')).toBeNull();
    expect(client.deleteInstall).toHaveBeenCalledWith(REPO);
  });

  it('checks every store install for updates in one batch and announces the result', async () => {
    const { installer, notify, v2, setHead } = await setup();
    await installer.installRelease({ repoId: REPO, releaseId: R1 });
    setHead(v2);
    const updates = await installer.checkUpdates();
    expect(updates).toEqual({
      'glass-blur': { state: 'ok', latest: { releaseId: R2, version: '1.1.0' }, currentYanked: false, ownerPublisherId: OWNER }
    });
    expect(notify).toHaveBeenLastCalledWith(updates);
  });
});
