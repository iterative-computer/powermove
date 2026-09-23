import * as fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { afterEach, describe, expect, it, vi } from 'vitest';
import { decodeLoose, hashObject, parseCommit } from '@powermove/registry/git';
import { snapshot } from '@powermove/registry/snapshot';
import { ApiError, type ExtensionDetailDto, type ListingDto, type MeDto, type ReleaseDto } from '@powermove/registry/wire';
import type { MessageBoxOptions } from 'electron';

import type { ExtensionRecord } from '../../shared/extensions';
import { createProvenanceStore, type ProvenanceOrigin } from './provenance';
import {
  batchParts,
  createPublisher,
  manifestText,
  type PublishApi,
  type PutReleaseBody,
  type UploadPart
} from './publish';

const MINE = '66666666-6666-4666-8666-666666666666';
const THEIRS = '22222222-2222-4222-8222-222222222222';
const THEIR_REPO = '11111111-1111-4111-8111-111111111111';
const THEIR_RELEASE = '33333333-3333-4333-8333-333333333333';
const MY_REPO = '77777777-7777-4777-8777-777777777777';
const MY_RELEASE = '88888888-8888-4888-8888-888888888888';
const ORIGIN_COMMIT = 'c'.repeat(40);
const NOW = Date.UTC(2026, 8, 23, 12, 0, 0);

const roots: string[] = [];
afterEach(async () => {
  await Promise.all(roots.splice(0).map((dir) => fs.rm(dir, { recursive: true, force: true })));
});

const text = (value: string): Uint8Array => new TextEncoder().encode(value);

const me: MeDto = {
  user: { id: '55555555-5555-4555-8555-555555555555', name: 'Jude', email: 'jude@example.com', image: null },
  publisher: { id: MINE, handle: 'jude', tombstoned: false },
  settings: { rememberInstalls: true }
};

const manifestRaw = { id: 'glass-blur', name: 'Glass blur', version: '1.0.0', apiVersion: 2, description: 'Frosted glass.', contributes: ['effects'] };

function files(body = 'export default {}\n'): Record<string, string> {
  return {
    'manifest.json': manifestText(manifestRaw),
    'index.ts': body,
    'shaders/glass.frag': 'void main() {}\n'
  };
}

function listing(repoId: string): ListingDto {
  return {
    repoId, owner: { id: MINE, handle: 'jude', tombstoned: false }, slug: 'glass-blur', name: 'Glass blur', tagline: 'Frosted glass.',
    category: 'effects', iconUrl: null, visibility: 'public', latest: null, installCount: 0, forkCount: 0, licence: 'MIT',
    forkedFrom: null, createdAt: '2026-09-23T12:00:00.000Z', updatedAt: '2026-09-23T12:00:00.000Z'
  };
}

function release(body: PutReleaseBody, treeSha: string): ReleaseDto {
  return {
    id: MY_RELEASE, repoId: MY_REPO, version: body.version, commitSha: body.commitSha, treeSha, tarSha256: 'a'.repeat(64),
    apiVersion: 2, fileCount: 3, sizeBytes: 100, notes: body.notes ?? null, publishedAt: '2026-09-23T12:00:00.000Z', yankedAt: null,
    basedOnReleaseId: null,
    manifest: { id: 'glass-blur', name: 'Glass blur', version: body.version, apiVersion: 2, contributes: ['effects'], vars: [], forkedFrom: null, description: null }
  };
}

interface Uploaded { sha: string; type: string; body: Uint8Array }

/** A registry in memory: every object missing until uploaded, every put accepted unless told otherwise. */
function fakeApi(options: { detail?: ExtensionDetailDto | null; reject?: boolean; put?: (body: PutReleaseBody) => void } = {}) {
  const stored = new Map<string, Uploaded>();
  const uploads: UploadPart[][] = [];
  const puts: PutReleaseBody[] = [];
  const api = {
    detail: vi.fn(async (_handle: string, _slug: string): Promise<ExtensionDetailDto> => {
      if (!options.detail) throw new ApiError({ error: 'not_found' });
      return options.detail;
    }),
    missing: vi.fn(async (body: Parameters<PublishApi['missing']>[0]) => ({ missing: body.shas.filter((sha) => !stored.has(sha)) })),
    upload: vi.fn(async (parts: UploadPart[]) => {
      uploads.push(parts);
      if (options.reject) return { stored: [], present: [], rejected: [{ sha: parts[0]!.sha, code: 'hash_mismatch' as const }] };
      for (const part of parts) {
        const decoded = await decodeLoose(part.bytes, 4 * 1024 * 1024);
        stored.set(part.sha, { sha: part.sha, type: decoded.type, body: decoded.body });
      }
      return { stored: parts.map((part) => part.sha), present: [], rejected: [] };
    }),
    putRelease: vi.fn(async (_handle: string, _slug: string, body: PutReleaseBody) => {
      puts.push(body);
      options.put?.(body);
      const commit = stored.get(body.commitSha);
      if (!commit) throw new ApiError({ error: 'commit_missing' });
      const tree = parseCommit(commit.body).tree;
      return { repo: listing(MY_REPO), release: release(body, tree) };
    }),
    yank: vi.fn(async () => { throw new Error('not used'); }),
    myRepos: vi.fn(async () => ({ items: [] }))
  } satisfies PublishApi;
  return { api, stored, uploads, puts };
}

async function setup(input: {
  folder?: Record<string, string>;
  origin?: Partial<ProvenanceOrigin> | null;
  api?: ReturnType<typeof fakeApi>;
  confirm?: (options: MessageBoxOptions) => Promise<boolean>;
} = {}) {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'powermove-publish-'));
  roots.push(root);
  const userDir = path.join(root, 'extensions');
  const folder = path.join(userDir, 'glass-blur');
  for (const [file, body] of Object.entries(input.folder ?? files())) {
    await fs.mkdir(path.dirname(path.join(folder, file)), { recursive: true });
    await fs.writeFile(path.join(folder, file), body);
  }
  const provenance = createProvenanceStore(root);
  let origin: ProvenanceOrigin | undefined;
  if (input.origin !== null && input.origin !== undefined) {
    const installed = await snapshot(Object.entries(files()).map(([file, body]) => ({ path: file, bytes: text(body) })));
    origin = {
      repoId: THEIR_REPO, releaseId: THEIR_RELEASE, coordinate: 'mara/glass-blur', version: '1.0.0',
      treeSha: installed.treeSha, commitSha: ORIGIN_COMMIT, ownerPublisherId: THEIRS, ...input.origin
    };
    const installedOrigin = origin;
    await provenance.update('glass-blur', () => ({
      localId: 'glass-blur', envKey: installedOrigin.repoId, origin: installedOrigin,
      upstream: { releaseId: installedOrigin.releaseId, treeSha: installedOrigin.treeSha }
    }));
  }
  const records: ExtensionRecord[] = [{
    id: 'glass-blur', scope: 'user', dir: folder, enabled: true, bundleUrl: null, bundleHash: null, health: { state: 'ok' }, updatedAt: 1,
    manifest: { id: 'glass-blur', name: 'Glass blur', version: '1.0.0', apiVersion: 2 }
  }];
  const fake = input.api ?? fakeApi();
  const confirm = vi.fn(input.confirm ?? (async () => true));
  const notifyLibrary = vi.fn();
  const publisher = createPublisher({
    registry: { userDir, list: () => records, refresh: async () => undefined, emitChanged: () => undefined },
    provenance,
    api: fake.api,
    me: () => me,
    signedIn: () => true,
    confirm,
    notifyLibrary,
    now: () => NOW,
    log: () => undefined
  });
  return { root, folder, provenance, publisher, confirm, notifyLibrary, origin, ...fake };
}

const firstForm = {
  version: '1.0.0',
  listing: { name: 'Glass blur', tagline: 'Frosted glass.', category: 'effects' as const, licence: 'MIT' as const },
  visibility: 'public' as const,
  waivers: []
};

/** The uploaded commit, decoded. */
function commitOf(stored: Map<string, Uploaded>, sha: string) {
  const object = stored.get(sha);
  if (!object || object.type !== 'commit') throw new Error('commit not uploaded');
  return parseCommit(object.body);
}

describe('publish: prepare', () => {
  it('plans a folder made here as a first publish with no parent', async () => {
    const { publisher, stored, puts } = await setup();
    const plan = await publisher.prepare('glass-blur');
    expect(plan).toMatchObject({
      coordinate: 'jude/glass-blur', version: '1.0.0', suggestedVersion: '1.0.0', lastVersion: null, firstPublish: true,
      isFork: false, fileCount: 3, blockedFindings: [], waivableFindings: [],
      listing: { name: 'Glass blur', tagline: 'Frosted glass.', category: 'effects', licence: 'MIT' }
    });
    expect(plan.parent).toBeUndefined();
    expect(plan.origin).toBeUndefined();

    await expect(publisher.publish('glass-blur', firstForm)).resolves.toMatchObject({ published: true, coordinate: 'jude/glass-blur', version: '1.0.0' });
    const commit = commitOf(stored, puts[0]!.commitSha);
    expect(commit.parents).toEqual([]);
    expect(commit.author).toEqual({ name: 'jude', email: 'jude@users.trypowermove.com', time: Math.floor(NOW / 1000), tz: '+0000' });
    expect(commit.message).toBe('jude/glass-blur 1.0.0');
    expect(puts[0]).not.toHaveProperty('originReleaseId');
  });

  it('refuses a fork identical to what was installed, before any network call', async () => {
    const { publisher, api } = await setup({ origin: {} });
    await expect(publisher.prepare('glass-blur')).rejects.toMatchObject({ body: { error: 'same_as_origin' } });
    expect(api.detail).not.toHaveBeenCalled();
    expect(api.missing).not.toHaveBeenCalled();
  });

  it('lists hard findings as blocked and soft ones as waivable, with any in-file reason', async () => {
    const soft = 'const a = "Zq9vXk2Lr7Tn4Wb8Yc1Hd6Jf3Gs5Pm0QaZ"; // powermove-secret-ok: test fixture';
    const { publisher } = await setup({ folder: { ...files(), 'index.ts': `${soft}\nconst k = "AKIAABCDEFGHIJKLMNOP";\n` } });
    const plan = await publisher.prepare('glass-blur');
    expect(plan.blockedFindings).toEqual([{ path: 'index.ts', line: 2, kind: 'aws_access_key' }]);
    expect(plan.waivableFindings).toEqual([{ path: 'index.ts', line: 1, kind: 'high_entropy', reason: 'test fixture' }]);
  });
});

describe('publish: fork at publish', () => {
  it('writes forkedFrom into the uploaded manifest only, and records published and upstream', async () => {
    const { publisher, folder, stored, puts, api, provenance, notifyLibrary } = await setup({ origin: {}, folder: files('export default { tint: 1 }\n') });
    const before = await fs.readFile(path.join(folder, 'manifest.json'));

    const plan = await publisher.prepare('glass-blur');
    expect(plan).toMatchObject({ isFork: true, origin: { coordinate: 'mara/glass-blur', version: '1.0.0' }, parent: ORIGIN_COMMIT, firstPublish: true });
    await expect(publisher.publish('glass-blur', firstForm)).resolves.toMatchObject({ published: true, repoId: MY_REPO, releaseId: MY_RELEASE });

    // The folder is untouched, byte for byte.
    expect(Buffer.compare(await fs.readFile(path.join(folder, 'manifest.json')), before)).toBe(0);
    // The uploaded manifest differs only by the injected forkedFrom.
    const expected = text(manifestText({ ...manifestRaw, forkedFrom: 'mara/glass-blur@1.0.0' }));
    const uploadedManifest = [...stored.values()].find((object) => object.type === 'blob' && new TextDecoder().decode(object.body).includes('"forkedFrom"'));
    expect(uploadedManifest && Buffer.compare(Buffer.from(uploadedManifest.body), Buffer.from(expected))).toBe(0);

    expect(api.missing.mock.calls[0]![0]).toMatchObject({ originReleaseId: THEIR_RELEASE });
    expect(api.missing.mock.calls[0]![0]).not.toHaveProperty('repoId');
    expect(puts[0]).toMatchObject({ originReleaseId: THEIR_RELEASE, visibility: 'public' });
    expect(commitOf(stored, puts[0]!.commitSha).parents).toEqual([ORIGIN_COMMIT]);

    const record = await provenance.get('glass-blur');
    const localTree = (await snapshot(Object.entries(files('export default { tint: 1 }\n')).map(([file, body]) => ({ path: file, bytes: text(body) })))).treeSha;
    expect(record?.published).toEqual({
      repoId: MY_REPO, releaseId: MY_RELEASE, version: '1.0.0', commitSha: puts[0]!.commitSha, ownerPublisherId: MINE,
      coordinate: 'jude/glass-blur', publishedTreeSha: commitOf(stored, puts[0]!.commitSha).tree, localTreeSha: localTree
    });
    expect(record?.published?.publishedTreeSha).not.toBe(localTree);
    expect(record?.upstream).toEqual({ releaseId: THEIR_RELEASE, treeSha: record?.origin?.treeSha });
    expect(notifyLibrary).toHaveBeenCalledTimes(1);
  });
});

describe('publish: confirmation and upload', () => {
  it('cancelling the confirmation aborts before any network call', async () => {
    const { publisher, api } = await setup({ confirm: async () => false });
    await publisher.prepare('glass-blur');
    const calls = api.detail.mock.calls.length;
    await expect(publisher.publish('glass-blur', firstForm)).resolves.toEqual({ published: false });
    expect(api.detail.mock.calls.length).toBe(calls);
    expect(api.missing).not.toHaveBeenCalled();
    expect(api.upload).not.toHaveBeenCalled();
    expect(api.putRelease).not.toHaveBeenCalled();
  });

  it('shows the tree hash it uploads, and every part is named by the hash of what it holds', async () => {
    const { publisher, confirm, stored, uploads, puts } = await setup();
    await publisher.prepare('glass-blur');
    await publisher.publish('glass-blur', { ...firstForm, version: '1.0.1' });
    const shown = confirm.mock.calls[0]![0];
    expect(shown.message).toBe('Publish jude/glass-blur 1.0.1?');
    const tree = commitOf(stored, puts[0]!.commitSha).tree;
    expect(shown.detail).toBe(`Snapshot ${tree.slice(0, 12)}. 3 files, ${shown.detail?.split(', ')[1]}`);
    expect(shown.buttons).toEqual(['Publish', 'Cancel']);
    // The same tree the dialog named is the one committed, and it carries the new version.
    const manifest = [...stored.values()].find((object) => object.type === 'blob' && new TextDecoder().decode(object.body).includes('"version": "1.0.1"'));
    expect(manifest).toBeDefined();
    for (const part of uploads.flat()) {
      const decoded = await decodeLoose(part.bytes, 4 * 1024 * 1024);
      expect(await hashObject(decoded.type, decoded.body)).toBe(part.sha);
    }
  });

  it('uploads only what is missing, in batches the envelope allows', async () => {
    const parts = Array.from({ length: 65 }, (_, index) => ({ sha: index.toString(16).padStart(40, '0'), bytes: new Uint8Array(10) }));
    expect(batchParts(parts).map((batch) => batch.length)).toEqual([64, 1]);
    expect(batchParts([{ sha: 'a', bytes: new Uint8Array(900) }, { sha: 'b', bytes: new Uint8Array(900) }], { parts: 64, bytes: 1500 }).length).toBe(2);

    // 63 extra files + manifest.json = 64 blobs, their tree and the commit: 66 objects, 2 requests.
    const folder = files();
    delete folder['shaders/glass.frag'];
    folder['index.ts'] = 'export default {}\n';
    for (let index = 0; index < 62; index++) folder[`part-${index}.ts`] = `export const n = ${index};\n`;
    const { publisher, uploads, api } = await setup({ folder });
    await publisher.prepare('glass-blur');
    await publisher.publish('glass-blur', firstForm);
    expect(uploads.map((batch) => batch.length)).toEqual([64, 2]);
    expect(api.upload).toHaveBeenCalledTimes(2);
  });

  it('aborts when the registry rejects an object', async () => {
    const { publisher, api } = await setup({ api: fakeApi({ reject: true }) });
    await publisher.prepare('glass-blur');
    await expect(publisher.publish('glass-blur', firstForm)).rejects.toMatchObject({ code: 'upload_rejected' });
    expect(api.putRelease).not.toHaveBeenCalled();
  });

  it('uploads again once when objects went missing before the put', async () => {
    let first = true;
    const fake = fakeApi({ put: () => { if (first) { first = false; throw new ApiError({ error: 'object_missing', shas: ['a'.repeat(40)] }); } } });
    const { publisher, api } = await setup({ api: fake });
    await publisher.prepare('glass-blur');
    await expect(publisher.publish('glass-blur', firstForm)).resolves.toMatchObject({ published: true });
    expect(api.missing).toHaveBeenCalledTimes(2);
    expect(api.putRelease).toHaveBeenCalledTimes(2);
  });

  it('explains head_moved in one sentence', async () => {
    const fake = fakeApi({ put: () => { throw new ApiError({ error: 'head_moved', head: 'b'.repeat(40) }); } });
    const { publisher } = await setup({ api: fake });
    await publisher.prepare('glass-blur');
    await expect(publisher.publish('glass-blur', firstForm)).rejects.toMatchObject({
      body: {
        error: 'head_moved',
        head: 'b'.repeat(40),
        detail: 'This extension was published from another Mac since you last published here. Reinstall that version from the store first, then make your changes and publish.'
      }
    });
  });

  it('refuses blocked findings and missing reasons before anything is sent', async () => {
    const soft = 'const a = "Zq9vXk2Lr7Tn4Wb8Yc1Hd6Jf3Gs5Pm0QaZ";';
    const { publisher, api, confirm } = await setup({ folder: { ...files(), 'index.ts': `${soft}\n` } });
    await publisher.prepare('glass-blur');
    await expect(publisher.publish('glass-blur', firstForm)).rejects.toMatchObject({ code: 'folder_invalid' });
    expect(confirm).not.toHaveBeenCalled();
    await expect(publisher.publish('glass-blur', { ...firstForm, waivers: [{ path: 'index.ts', line: 1, reason: 'test fixture' }] }))
      .resolves.toMatchObject({ published: true });
    expect(api.putRelease.mock.calls[0]![2].waivers).toEqual([{ path: 'index.ts', line: 1, reason: 'test fixture' }]);
  });

  it('writes a new version back into the folder after publishing an update', async () => {
    const detail: ExtensionDetailDto = {
      ...listing(MY_REPO), about: 'About it.', moderation: 'none',
      releases: [{ ...release({ version: '1.0.0', commitSha: 'd'.repeat(40), listing: { name: 'Glass blur', tagline: '', category: 'effects' }, waivers: [] }, 'e'.repeat(40)) }]
    };
    const { publisher, provenance, folder, puts, stored } = await setup({ api: fakeApi({ detail }) });
    await provenance.update('glass-blur', () => ({
      localId: 'glass-blur', envKey: 'local:1',
      published: { repoId: MY_REPO, releaseId: MY_RELEASE, version: '1.0.0', commitSha: 'd'.repeat(40), ownerPublisherId: MINE }
    }));
    const plan = await publisher.prepare('glass-blur');
    expect(plan).toMatchObject({ firstPublish: false, lastVersion: '1.0.0', suggestedVersion: '1.0.1', parent: 'd'.repeat(40), listing: { about: 'About it.' } });
    await expect(publisher.publish('glass-blur', { version: '1.0.0', waivers: [] })).rejects.toMatchObject({ code: 'folder_invalid' });
    await publisher.publish('glass-blur', { version: '1.0.1', notes: 'Sharper.', waivers: [] });
    expect(puts[0]).toMatchObject({ version: '1.0.1', notes: 'Sharper.', listing: { name: 'Glass blur', about: 'About it.' } });
    expect(puts[0]).not.toHaveProperty('visibility');
    expect(commitOf(stored, puts[0]!.commitSha).parents).toEqual(['d'.repeat(40)]);
    const written = JSON.parse(await fs.readFile(path.join(folder, 'manifest.json'), 'utf8')) as { version: string };
    expect(written.version).toBe('1.0.1');
    const record = await provenance.get('glass-blur');
    const now = (await snapshot(await Promise.all(['manifest.json', 'index.ts', 'shaders/glass.frag'].map(async (file) => ({ path: file, bytes: new Uint8Array(await fs.readFile(path.join(folder, file))) }))))).treeSha;
    expect(record?.published?.localTreeSha).toBe(now);
    expect(record?.published?.publishedTreeSha).toBe(now);
  });
});
