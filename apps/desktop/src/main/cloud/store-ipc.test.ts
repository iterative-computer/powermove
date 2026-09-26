import { describe, expect, it, vi } from 'vitest';
import { ApiError, type MeDto } from '@powermove/registry/wire';

import type { ExtensionRecord } from '../../shared/extensions';
import { STORE_IPC } from '../../shared/store-ipc';
import { StoreLocalError, type StoreInstaller } from './install';
import type { ProvenanceFile, ProvenanceStore } from './provenance';
import type { Publisher } from './publish';
import type { StoreClient } from './store-client';
import { buildLibrary, categoryFor, registerStoreIpc, storeResult, storeSchemas } from './store-ipc';

const REPO = '11111111-1111-4111-8111-111111111111';
const R1 = '33333333-3333-4333-8333-333333333333';
const R2 = '44444444-4444-4444-8444-444444444444';
const MINE = '66666666-6666-4666-8666-666666666666';
const THEIRS = '22222222-2222-4222-8222-222222222222';

describe('store IPC schemas', () => {
  it('accepts well-formed requests', () => {
    expect(storeSchemas['store:browse'].safeParse(undefined).success).toBe(true);
    expect(storeSchemas['store:extensions'].safeParse({ category: 'tools', q: 'blur', sort: 'new', cursor: 'abc_-1' }).success).toBe(true);
    expect(storeSchemas['store:detail'].safeParse({ handle: 'mara', slug: 'glass-blur' }).success).toBe(true);
    expect(storeSchemas['store:file'].safeParse({ handle: 'mara', slug: 'glass-blur', version: '1.0.0', path: 'shaders/glass.frag' }).success).toBe(true);
    expect(storeSchemas['store:install'].safeParse({ repoId: REPO, releaseId: R1 }).success).toBe(true);
    expect(storeSchemas['store:compare'].safeParse({ base: R1, head: R2 }).success).toBe(true);
    expect(storeSchemas['store:uninstall'].safeParse({ localId: 'glass-blur' }).success).toBe(true);
    expect(storeSchemas['store:publish-prepare'].safeParse({ localId: 'glass-blur' }).success).toBe(true);
    expect(storeSchemas['store:publish'].safeParse({ localId: 'glass-blur', form: { version: '1.2.0', waivers: [] } }).success).toBe(true);
    expect(storeSchemas['store:publish'].safeParse({
      localId: 'glass-blur',
      form: {
        version: '1.0.0', notes: 'First.', visibility: 'unlisted', iconPng: 'iVBORw0KGgo=',
        listing: { name: 'Glass blur', tagline: 'Frosted glass.', category: 'effects', licence: 'Apache-2.0' },
        waivers: [{ path: 'index.ts', line: 3, reason: 'test fixture' }]
      }
    }).success).toBe(true);
    expect(storeSchemas['store:yank'].safeParse({ repoId: REPO, version: '1.2.0' }).success).toBe(true);
  });

  it('refuses anything else', () => {
    const bad: Array<[keyof typeof storeSchemas, unknown]> = [
      ['store:browse', { extra: 1 }],
      ['store:extensions', { category: 'widgets' }],
      ['store:extensions', { q: 'x'.repeat(500) }],
      ['store:extensions', { cursor: '../../etc' }],
      ['store:extensions', { extra: true }],
      ['store:detail', { handle: 'Mara', slug: 'glass-blur' }],
      ['store:detail', { handle: 'mara', slug: '../x' }],
      ['store:file', { handle: 'mara', slug: 'glass-blur', version: '1.0.0', path: '../secret' }],
      ['store:file', { handle: 'mara', slug: 'glass-blur', version: '1.0.0', path: '/etc/passwd' }],
      ['store:file', { handle: 'mara', slug: 'glass-blur', version: '1.0', path: 'a.ts' }],
      ['store:install', { repoId: 'not-a-uuid', releaseId: R1 }],
      ['store:install', { repoId: REPO }],
      ['store:update', { localId: '.staging' }],
      ['store:uninstall', { localId: '../glass-blur' }],
      ['store:library', 'x'],
      ['store:publish-prepare', { localId: '../x' }],
      ['store:publish-prepare', { localId: 'glass-blur', extra: true }],
      ['store:publish', { localId: 'glass-blur', form: { version: '1.2', waivers: [] } }],
      ['store:publish', { localId: 'glass-blur', form: { version: '1.2.0' } }],
      ['store:publish', { localId: 'glass-blur', form: { version: '1.2.0', waivers: [], token: 'x' } }],
      ['store:publish', { localId: 'glass-blur', form: { version: '1.2.0', waivers: [{ path: 'a.ts', line: 1, reason: 'ok' }] } }],
      ['store:publish', { localId: 'glass-blur', form: { version: '1.2.0', waivers: [{ path: 'a.ts', line: 0, reason: 'fixture' }] } }],
      ['store:publish', { localId: 'glass-blur', form: { version: '1.2.0', waivers: [{ path: 'a.ts', line: 1, reason: 'x'.repeat(201) }] } }],
      ['store:publish', { localId: 'glass-blur', form: { version: '1.2.0', waivers: [], notes: 'x'.repeat(4001) } }],
      ['store:publish', { localId: 'glass-blur', form: { version: '1.2.0', waivers: [], visibility: 'private' } }],
      ['store:publish', { localId: 'glass-blur', form: { version: '1.2.0', waivers: [], iconPng: 'not base64!' } }],
      ['store:publish', { localId: 'glass-blur', form: { version: '1.2.0', waivers: [], iconPng: 'A'.repeat(400_000) } }],
      ['store:publish', { localId: 'glass-blur', form: { version: '1.2.0', waivers: [], listing: { name: 'x', tagline: 't'.repeat(161), category: 'effects', licence: 'MIT' } } }],
      ['store:publish', { localId: 'glass-blur', form: { version: '1.2.0', waivers: [], listing: { name: 'x', tagline: '', category: 'effects', licence: 'WTFPL' } } }],
      ['store:publish', { localId: 'glass-blur', form: { version: '1.2.0', waivers: [], listing: { name: ' ', tagline: '', category: 'effects', licence: 'MIT' } } }],
      ['store:yank', { repoId: 'x', version: '1.0.0' }],
      ['store:yank', { repoId: REPO, version: 'latest' }]
    ];
    for (const [channel, payload] of bad) {
      expect(storeSchemas[channel].safeParse(payload).success, `${channel} ${JSON.stringify(payload)}`).toBe(false);
    }
  });
});

describe('store IPC handlers', () => {
  function register(trusted = true, overrides: Partial<Parameters<typeof registerStoreIpc>[1]> = {}) {
    const handlers = new Map<string, (event: unknown, payload: unknown) => Promise<unknown>>();
    const ipcMain = { handle: vi.fn((channel: string, fn: (event: unknown, payload: unknown) => Promise<unknown>) => { handlers.set(channel, fn); }) };
    const store = { detail: vi.fn(async () => { throw new ApiError({ error: 'gone', reason: 'removed' }); }) } as unknown as StoreClient;
    const installer = {
      installRelease: vi.fn(async () => { throw new StoreLocalError('integrity', 'nope'); }),
      uninstall: vi.fn(async (_id: string, remove: (id: string) => Promise<boolean>) => ({ removed: await remove('glass-blur') })),
      updates: () => ({}),
      isModified: async () => false,
      localTree: async () => null
    } as unknown as StoreInstaller;
    const publisher: Publisher = {
      prepare: vi.fn(async () => { throw new ApiError({ error: 'forbidden', detail: 'Choose a handle first.' }); }),
      publish: vi.fn(async () => ({ published: false as const })),
      yank: vi.fn(async (target: string | { repoId: string; version: string }) => ({ version: typeof target === 'string' ? '1.0.0' : target.version }))
    };
    const provenance = { read: async () => ({}) } as unknown as ProvenanceStore;
    const removeExtension = vi.fn(async () => true);
    registerStoreIpc(ipcMain, {
      store, installer, publisher, provenance, removeExtension,
      registry: { list: () => [], refresh: async () => undefined, emitChanged: () => undefined },
      me: () => null,
      confirmTrust: async () => false,
      isTrusted: () => trusted,
      ...overrides
    });
    return { handlers, store, installer, publisher, removeExtension };
  }

  it('registers every channel', () => {
    const { handlers } = register();
    const events: string[] = [STORE_IPC.updatesChanged, STORE_IPC.publishProgress, STORE_IPC.libraryChanged];
    const expected = Object.values(STORE_IPC).filter((channel) => !events.includes(channel)).sort();
    expect([...handlers.keys()].sort()).toEqual(expected);
  });

  it('throws for untrusted senders and malformed payloads', async () => {
    await expect(register(false).handlers.get(STORE_IPC.library)!({}, undefined)).rejects.toThrow(/untrusted/);
    await expect(register().handlers.get(STORE_IPC.install)!({}, { repoId: 'x' })).rejects.toThrow();
  });

  it('returns registry and local failures as results', async () => {
    const { handlers } = register();
    await expect(handlers.get(STORE_IPC.detail)!({}, { handle: 'mara', slug: 'glass-blur' }))
      .resolves.toEqual({ ok: false, error: { error: 'gone', reason: 'removed' } });
    await expect(handlers.get(STORE_IPC.install)!({}, { repoId: REPO, releaseId: R1 }))
      .resolves.toEqual({ ok: false, error: { error: 'integrity', detail: 'nope' } });
  });

  it('publishes and yanks through the publisher, with only the fields it asked for', async () => {
    const { handlers, publisher } = register();
    await expect(handlers.get(STORE_IPC.publishPrepare)!({}, { localId: 'glass-blur' }))
      .resolves.toEqual({ ok: false, error: { error: 'forbidden', detail: 'Choose a handle first.' } });
    await expect(handlers.get(STORE_IPC.publish)!({}, { localId: 'glass-blur', form: { version: '1.1.0', waivers: [], notes: 'Faster.' } }))
      .resolves.toEqual({ ok: true, value: { published: false } });
    expect(publisher.publish).toHaveBeenCalledWith('glass-blur', { version: '1.1.0', waivers: [], notes: 'Faster.' });
    await expect(handlers.get(STORE_IPC.yank)!({}, { repoId: REPO, version: '1.1.0' })).resolves.toEqual({ ok: true, value: { version: '1.1.0' } });
    expect(publisher.yank).toHaveBeenCalledWith({ repoId: REPO, version: '1.1.0' });
    await expect(register(false).handlers.get(STORE_IPC.publish)!({}, { localId: 'glass-blur', form: { version: '1.1.0', waivers: [] } })).rejects.toThrow(/untrusted/);
  });

  it('uninstalls through the removal path for the asking window', async () => {
    const { handlers, removeExtension } = register();
    const event = { sender: 'window-1' };
    await expect(handlers.get(STORE_IPC.uninstall)!(event, { localId: 'glass-blur' })).resolves.toEqual({ ok: true, value: { removed: true } });
    expect(removeExtension).toHaveBeenCalledWith(event, 'glass-blur');
  });

  describe('trust', () => {
    const theirs = { repoId: REPO, releaseId: R1, coordinate: 'mara/glass-blur', version: '1.0.0', treeSha: 't', commitSha: 'c', ownerPublisherId: THEIRS };
    const full: ExtensionRecord = {
      id: 'glass-blur', scope: 'user', dir: '/x/glass-blur', enabled: true, bundleUrl: null, bundleHash: null,
      health: { state: 'needs-trust' }, updatedAt: 1, trust: 'store',
      manifest: { id: 'glass-blur', name: 'Glass blur', version: '1.0.0', apiVersion: 3, permissions: ['full-access', 'network'] }
    };
    function setup(entry: Record<string, unknown> | null, answer: boolean) {
      let current = entry;
      const provenance = {
        read: async () => ({}),
        get: vi.fn(async () => current),
        update: vi.fn(async (_id: string, mutate: (value: unknown) => unknown) => { current = mutate(current) as typeof current; return current; })
      } as unknown as ProvenanceStore;
      const confirmTrust = vi.fn(async () => answer);
      const refresh = vi.fn(async () => undefined);
      const emitChanged = vi.fn();
      const { handlers } = register(true, {
        provenance, confirmTrust, now: () => Date.UTC(2026, 8, 23),
        registry: { list: () => [full], refresh, emitChanged }
      });
      return { handlers, provenance, confirmTrust, refresh, emitChanged, current: () => current };
    }

    it('writes nothing unless the native dialog says Trust', async () => {
      const cancelled = setup({ localId: 'glass-blur', envKey: REPO, origin: theirs }, false);
      await expect(cancelled.handlers.get(STORE_IPC.trust)!({}, { localId: 'glass-blur' })).resolves.toEqual({ ok: true, value: { localId: 'glass-blur', trusted: false } });
      expect(cancelled.confirmTrust).toHaveBeenCalledWith('Glass blur');
      expect(cancelled.provenance.update).not.toHaveBeenCalled();
      expect(cancelled.refresh).not.toHaveBeenCalled();

      const agreed = setup({ localId: 'glass-blur', envKey: REPO, origin: theirs }, true);
      await expect(agreed.handlers.get(STORE_IPC.trust)!({}, { localId: 'glass-blur' })).resolves.toEqual({ ok: true, value: { localId: 'glass-blur', trusted: true } });
      expect(agreed.current()).toMatchObject({ trusted: { at: '2026-09-23T00:00:00.000Z', permissions: ['full-access', 'network'] } });
      expect(agreed.refresh).toHaveBeenCalledWith(['glass-blur']);
      expect(agreed.emitChanged).toHaveBeenCalledWith({ ids: ['glass-blur'], reason: 'reload' });
    });

    it('never asks for folders made here, and revokes without a dialog', async () => {
      const local = setup({ localId: 'glass-blur', envKey: 'local:1' }, true);
      await expect(local.handlers.get(STORE_IPC.trust)!({}, { localId: 'glass-blur' })).resolves.toMatchObject({ ok: false, error: { error: 'not_installed' } });
      expect(local.confirmTrust).not.toHaveBeenCalled();

      const trusted = setup({ localId: 'glass-blur', envKey: REPO, origin: theirs, trusted: { at: 'x', permissions: ['full-access'] } }, false);
      await expect(trusted.handlers.get(STORE_IPC.untrust)!({}, { localId: 'glass-blur' })).resolves.toEqual({ ok: true, value: { localId: 'glass-blur', trusted: false } });
      expect(trusted.confirmTrust).not.toHaveBeenCalled();
      expect(trusted.current()).not.toHaveProperty('trusted');
      expect(trusted.emitChanged).toHaveBeenCalledWith({ ids: ['glass-blur'], reason: 'reload' });
    });

    it('refuses untrusted senders and bad ids before any dialog', async () => {
      const { confirmTrust } = setup({ localId: 'glass-blur', envKey: REPO, origin: theirs }, true);
      await expect(register(false, { confirmTrust }).handlers.get(STORE_IPC.trust)!({}, { localId: 'glass-blur' })).rejects.toThrow(/untrusted/);
      await expect(register(true, { confirmTrust }).handlers.get(STORE_IPC.trust)!({}, { localId: '../x' })).rejects.toThrow();
      expect(confirmTrust).not.toHaveBeenCalled();
    });
  });

  it('never leaks an unexpected error', async () => {
    const error = vi.spyOn(console, 'error').mockImplementation(() => undefined);
    await expect(storeResult(async () => { throw new Error('/Users/jude/secret'); }))
      .resolves.toEqual({ ok: false, error: { error: 'internal', detail: 'Something went wrong on this Mac. Try again.' } });
    error.mockRestore();
  });
});

describe('library', () => {
  const record = (id: string, scope: ExtensionRecord['scope'], extra: Partial<NonNullable<ExtensionRecord['manifest']>> = {}): ExtensionRecord => ({
    id, scope, dir: `/x/${id}`, enabled: true, bundleUrl: null, bundleHash: null, health: { state: 'ok' }, updatedAt: 1,
    manifest: { id, name: id, version: '1.0.0', apiVersion: 2, ...extra }
  });
  const me = (publisherId: string | null): MeDto => ({
    user: { id: '55555555-5555-4555-8555-555555555555', name: null, email: 'j@example.com', image: null },
    publisher: publisherId ? { id: publisherId, handle: 'jude', tombstoned: false } : null,
    settings: { rememberInstalls: true }
  });
  const origin = (owner: string) => ({ repoId: REPO, releaseId: R1, coordinate: 'mara/glass-blur', version: '1.0.0', treeSha: 't', commitSha: 'c', ownerPublisherId: owner });

  const records = [
    record('timeline', 'builtin', { contributes: ['panels'] }),
    record('glass-blur', 'user', { contributes: ['effects'] }),
    record('ease-lab', 'user', { contributes: ['inspector'] }),
    record('my-fork', 'user', { forkedFrom: 'mara/glass-blur@1.0.0' }),
    record('gone-one', 'user')
  ];
  const provenance: ProvenanceFile = {
    'glass-blur': { localId: 'glass-blur', envKey: REPO, origin: origin(THEIRS) },
    'ease-lab': { localId: 'ease-lab', envKey: 'local:1' },
    'my-fork': { localId: 'my-fork', envKey: REPO, origin: origin(THEIRS), published: { repoId: R2, releaseId: R2, version: '1.0.0', ownerPublisherId: MINE } },
    'gone-one': { localId: 'gone-one', envKey: REPO, origin: origin(THEIRS) }
  };
  const updates = {
    'glass-blur': { state: 'ok' as const, latest: { releaseId: R2, version: '1.1.0' }, currentYanked: false, ownerPublisherId: THEIRS },
    'gone-one': { state: 'removed' as const, latest: null, currentYanked: false, ownerPublisherId: null }
  };

  it('groups per the plan: published and mine, store installs, made here, built in', () => {
    const items = buildLibrary({ records, provenance, updates, me: me(MINE), modified: (id) => id === 'glass-blur' });
    const byId = Object.fromEntries(items.map((item) => [item.localId, item]));
    expect(byId['timeline']).toMatchObject({ group: 'builtin', maker: { builtin: true }, category: 'panels' });
    expect(byId['glass-blur']).toMatchObject({
      group: 'store', maker: { handle: 'mara' }, category: 'effects', modified: true,
      origin: { coordinate: 'mara/glass-blur', version: '1.0.0', repoId: REPO, releaseId: R1 },
      update: { version: '1.1.0', releaseId: R2, modified: true, state: 'available' }
    });
    expect(byId['ease-lab']).toMatchObject({ group: 'yours', maker: { you: true }, category: 'panels', update: null, modified: false });
    expect(byId['my-fork']).toMatchObject({ group: 'yours', maker: { you: true }, forkedFrom: 'mara/glass-blur@1.0.0', category: 'tools' });
    expect(byId['gone-one']).toMatchObject({ group: 'store', removed: true, update: null });
  });

  it('carries trust and permissions, from the record when main derived it', () => {
    const withPermissions = [
      ...records.slice(0, 1),
      { ...record('glass-blur', 'user', { apiVersion: 3, permissions: ['network', 'full-access'] }), trust: 'store-trusted' as const },
      ...records.slice(2)
    ];
    const byId = Object.fromEntries(buildLibrary({ records: withPermissions, provenance, updates, me: me(MINE), modified: () => false }).map((item) => [item.localId, item]));
    expect(byId['glass-blur']).toMatchObject({ trust: 'store-trusted', permissions: ['network', 'full-access'] });
    expect(byId['timeline']).toMatchObject({ trust: 'builtin', permissions: [] });
    expect(byId['ease-lab']).toMatchObject({ trust: 'local' });
    expect(byId['gone-one']).toMatchObject({ trust: 'store' });
    expect(byId['my-fork']).toMatchObject({ trust: 'local' });
  });

  it('puts a published folder back under the store when signed out', () => {
    const items = buildLibrary({ records, provenance, updates, me: null, modified: () => false });
    expect(items.find((item) => item.localId === 'my-fork')).toMatchObject({ group: 'store', maker: { handle: 'mara' } });
  });

  it('reports a staged modified update', () => {
    const staged: ProvenanceFile = {
      ...provenance,
      'glass-blur': { ...provenance['glass-blur']!, pendingUpdate: { releaseId: R2, version: '1.1.0', path: '/x/.updates/glass-blur' } }
    };
    const withCheck = buildLibrary({ records, provenance: staged, updates, me: null, modified: () => true });
    expect(withCheck.find((item) => item.localId === 'glass-blur')?.update).toEqual({ version: '1.1.0', releaseId: R2, modified: true, state: 'staged-for-merge' });
    const beforeCheck = buildLibrary({ records, provenance: staged, updates: {}, me: null, modified: () => true });
    expect(beforeCheck.find((item) => item.localId === 'glass-blur')?.update).toEqual({ version: '1.1.0', releaseId: R2, modified: true, state: 'staged-for-merge' });
  });

  it('says what publishing would do for each folder', () => {
    const folders: ProvenanceFile = {
      ...provenance,
      'my-fork': {
        localId: 'my-fork', envKey: REPO, origin: origin(THEIRS), upstream: { releaseId: R1, treeSha: 't' },
        published: { repoId: R2, releaseId: R2, version: '1.0.0', ownerPublisherId: MINE, coordinate: 'jude/my-fork', publishedTreeSha: 'p', localTreeSha: 'l' }
      }
    };
    const trees: Record<string, string> = { 'glass-blur': 'changed', 'my-fork': 'l', 'gone-one': 't' };
    const items = buildLibrary({ records, provenance: folders, updates, me: me(MINE), modified: (id) => trees[id] !== 't', tree: (id) => trees[id] });
    const byId = Object.fromEntries(items.map((item) => [item.localId, item]));
    expect(byId['timeline']?.publish).toBeUndefined();
    expect(byId['ease-lab']?.publish).toBe('first');
    expect(byId['glass-blur']?.publish).toBe('first');
    expect(byId['gone-one']?.publish).toBeNull();
    expect(byId['my-fork']).toMatchObject({
      publish: null,
      published: { coordinate: 'jude/my-fork', version: '1.0.0', releaseId: R2, repoId: R2 },
      fork: { coordinate: 'mara/glass-blur', version: '1.0.0', releaseId: R1, upstreamReleaseId: R1 }
    });
    trees['my-fork'] = 'edited';
    expect(buildLibrary({ records, provenance: folders, updates, me: me(MINE), modified: () => true, tree: (id) => trees[id] })
      .find((item) => item.localId === 'my-fork')?.publish).toBe('update');
    // Signed out, nothing is yours to publish.
    expect(buildLibrary({ records, provenance: folders, updates, me: null, modified: () => true, tree: (id) => trees[id] })
      .every((item) => !item.publish)).toBe(true);
  });

  it('reads a category from what an extension contributes', () => {
    expect(categoryFor(['hooks', 'transitions'])).toBe('transitions');
    expect(categoryFor(['keybindings'])).toBe('commands');
    expect(categoryFor([])).toBe('tools');
  });
});
