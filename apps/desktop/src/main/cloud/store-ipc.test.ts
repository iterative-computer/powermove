import { describe, expect, it, vi } from 'vitest';
import { ApiError, type MeDto } from '@powermove/registry/wire';

import type { ExtensionRecord } from '../../shared/extensions';
import { STORE_IPC } from '../../shared/store-ipc';
import { StoreLocalError, type StoreInstaller } from './install';
import type { ProvenanceFile, ProvenanceStore } from './provenance';
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
      ['store:library', 'x']
    ];
    for (const [channel, payload] of bad) {
      expect(storeSchemas[channel].safeParse(payload).success, `${channel} ${JSON.stringify(payload)}`).toBe(false);
    }
  });
});

describe('store IPC handlers', () => {
  function register(trusted = true) {
    const handlers = new Map<string, (event: unknown, payload: unknown) => Promise<unknown>>();
    const ipcMain = { handle: vi.fn((channel: string, fn: (event: unknown, payload: unknown) => Promise<unknown>) => { handlers.set(channel, fn); }) };
    const store = { detail: vi.fn(async () => { throw new ApiError({ error: 'gone', reason: 'removed' }); }) } as unknown as StoreClient;
    const installer = {
      installRelease: vi.fn(async () => { throw new StoreLocalError('integrity', 'nope'); }),
      uninstall: vi.fn(async (_id: string, remove: (id: string) => Promise<boolean>) => ({ removed: await remove('glass-blur') })),
      updates: () => ({}),
      isModified: async () => false
    } as unknown as StoreInstaller;
    const provenance = { read: async () => ({}) } as unknown as ProvenanceStore;
    const removeExtension = vi.fn(async () => true);
    registerStoreIpc(ipcMain, {
      store, installer, provenance, removeExtension,
      registry: { list: () => [] },
      me: () => null,
      isTrusted: () => trusted
    });
    return { handlers, store, installer, removeExtension };
  }

  it('registers every channel', () => {
    const { handlers } = register();
    const expected = Object.values(STORE_IPC).filter((channel) => channel !== STORE_IPC.updatesChanged).sort();
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

  it('uninstalls through the removal path for the asking window', async () => {
    const { handlers, removeExtension } = register();
    const event = { sender: 'window-1' };
    await expect(handlers.get(STORE_IPC.uninstall)!(event, { localId: 'glass-blur' })).resolves.toEqual({ ok: true, value: { removed: true } });
    expect(removeExtension).toHaveBeenCalledWith(event, 'glass-blur');
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

  it('reads a category from what an extension contributes', () => {
    expect(categoryFor(['hooks', 'transitions'])).toBe('transitions');
    expect(categoryFor(['keybindings'])).toBe('commands');
    expect(categoryFor([])).toBe('tools');
  });
});
