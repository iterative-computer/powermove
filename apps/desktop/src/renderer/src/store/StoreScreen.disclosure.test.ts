// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import type { LibraryItemDto } from '../../../shared/store-ipc';
import { installBridgeForTests, resetBridgeForTests } from '../kernel/bridge';
import StoreScreen from './StoreScreen.svelte';
import type { StorePM } from './data';
import * as account from '../cloud/account';

afterEach(() => { resetBridgeForTests(); vi.unstubAllGlobals(); vi.restoreAllMocks(); });

it('renders permission disclosure, trust status, and the Library trust action', async () => {
  vi.stubGlobal('matchMedia', () => ({ matches: true, addListener() {}, removeListener() {} }));
  HTMLElement.prototype.scrollTo = vi.fn();
  const item: LibraryItemDto = {
    localId: 'glass-blur', name: 'Glass blur', version: '1.0.0', category: 'effects', contributes: ['effects'], vars: [],
    health: { state: 'needs-trust' }, enabled: false, trust: 'store', permissions: ['network', 'full-access'],
    description: 'Blur effect', group: 'store', maker: { handle: 'mara' },
    origin: { coordinate: 'mara/glass-blur', version: '1.0.0', repoId: '11111111-1111-4111-8111-111111111111', releaseId: '22222222-2222-4222-8222-222222222222' },
    update: null, modified: false
  };
  const trust = vi.fn(async () => ({ ok: true, value: { localId: item.localId, trusted: false } }));
  installBridgeForTests({ extensionStore: {
    library: async () => [item],
    browse: async () => ({ ok: true, value: { sections: [] } }),
    detail: async () => ({ ok: false, error: { error: 'not_found' } }),
    trust,
    onUpdatesChanged: () => () => {}, onLibraryChanged: () => () => {}
  } } as any);
  const target = document.createElement('div');
  document.body.append(target);
  const screen = mount(StoreScreen, { target, props: { PM: { bus: { emit() {} } } as unknown as StorePM } });
  try {
    flushSync(() => screen.open('library'));
    await vi.waitFor(() => expect(target.textContent).toContain('Needs full access'));
    expect(target.textContent).toContain('Glass blur');
    const row = target.querySelector('.st-item.is-library')!;
    expect(row.textContent).toContain('Trust…');
    (row.querySelector('.st-item-open') as HTMLButtonElement).click();
    flushSync();
    await vi.waitFor(() => expect(target.textContent).toContain('Uses the network'));
    expect(target.textContent).toContain('Needs full access to Powermove');
  } finally {
    await unmount(screen);
    target.remove();
  }
});

it('renders Trusted and offers Revoke Trust for a trusted Store install', async () => {
  vi.stubGlobal('matchMedia', () => ({ matches: true, addListener() {}, removeListener() {} }));
  HTMLElement.prototype.scrollTo = vi.fn();
  const item: LibraryItemDto = {
    localId: 'trusted-blur', name: 'Trusted blur', version: '1.0.0', category: 'effects', contributes: ['effects'], vars: [],
    health: { state: 'ok' }, enabled: true, trust: 'store-trusted', permissions: ['full-access'],
    description: 'Blur effect', group: 'store', maker: { handle: 'mara' }, update: null, modified: false
  };
  installBridgeForTests({ extensionStore: {
    library: async () => [item], browse: async () => ({ ok: true, value: { sections: [] } }),
    onUpdatesChanged: () => () => {}, onLibraryChanged: () => () => {}
  } } as any);
  const target = document.createElement('div');
  document.body.append(target);
  const screen = mount(StoreScreen, { target, props: { PM: { bus: { emit() {} } } as unknown as StorePM } });
  try {
    flushSync(() => screen.open('library'));
    await vi.waitFor(() => expect(target.querySelector('.st-item-status')?.textContent).toContain('Trusted'));
    (target.querySelector('.st-item-more') as HTMLButtonElement).click();
    expect(document.body.textContent).toContain('Revoke Trust');
  } finally { await unmount(screen); target.remove(); }
});


it('removes owner controls immediately on account switch despite local publication metadata', async () => {
  vi.stubGlobal('matchMedia', () => ({ matches: true, addListener() {}, removeListener() {} }));
  HTMLElement.prototype.scrollTo = vi.fn();
  let notify!: Parameters<typeof account.subscribeAccount>[0];
  const owner = { name: '@mara', handle: 'mara', email: 'mara@example.com', image: null };
  vi.spyOn(account, 'subscribeAccount').mockImplementation((listener) => {
    notify = listener;
    listener(owner, null);
    return () => {};
  });
  const repoId = '11111111-1111-4111-8111-111111111111';
  const releaseId = '22222222-2222-4222-8222-222222222222';
  const local: LibraryItemDto = {
    localId: 'glass-blur', name: 'Glass blur', version: '1.0.0', category: 'effects', contributes: ['effects'], vars: [],
    health: { state: 'ok' }, enabled: true, trust: 'local', permissions: [], description: 'Blur',
    group: 'yours', maker: { you: true }, update: null, modified: false,
    published: { coordinate: 'mara/glass-blur', repoId, releaseId, version: '1.0.0' }
  };
  const release = {
    id: releaseId, repoId, version: '1.0.0', apiVersion: 3, publishedAt: '2026-09-25T00:00:00Z', notes: 'First release', yankedAt: null,
    manifest: { contributes: ['effects'], vars: [], permissions: [] }
  };
  installBridgeForTests({ extensionStore: {
    library: async () => [local], browse: async () => ({ ok: true, value: { sections: [] } }),
    detail: async () => ({ ok: true, value: {
      repoId, owner: { handle: 'mara' }, slug: 'glass-blur', name: 'Glass blur', tagline: 'Blur', category: 'effects',
      latest: release, releases: [release], permissions: [], visibility: 'public', updatedAt: release.publishedAt,
      installCount: 0, forkCount: 0, iconUrl: null, about: null
    } }),
    tree: async () => ({ ok: true, value: { files: [] } }),
    onUpdatesChanged: () => () => {}, onLibraryChanged: () => () => {}
  } } as any);
  const target = document.createElement('div');
  document.body.append(target);
  const screen = mount(StoreScreen, { target, props: { PM: { bus: { emit() {} } } as unknown as StorePM } });
  try {
    flushSync(() => screen.open('library'));
    await vi.waitFor(() => expect(target.querySelector('.st-item-open')).not.toBeNull());
    (target.querySelector('.st-item-open') as HTMLButtonElement).click();
    await vi.waitFor(() => expect(target.querySelector('[aria-label="Withdraw 1.0.0…"]')).not.toBeNull());
    expect(target.textContent).toContain('runs in a sandbox');
    expect(target.textContent).not.toContain('same access as the app');
    flushSync(() => notify({ ...owner, name: '@jude', handle: 'jude' }, null));
    expect(target.querySelector('[aria-label="Withdraw 1.0.0…"]')).toBeNull();
    expect(target.querySelector('.st-facts dd')?.textContent).toBe('mara');
    flushSync(() => notify(null, null));
    expect(target.querySelector('[aria-label="Withdraw 1.0.0…"]')).toBeNull();
  } finally { await unmount(screen); target.remove(); }
});
