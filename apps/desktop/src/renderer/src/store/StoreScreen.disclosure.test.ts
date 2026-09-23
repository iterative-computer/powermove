// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import type { LibraryItemDto } from '../../../shared/store-ipc';
import { installBridgeForTests, resetBridgeForTests } from '../kernel/bridge';
import StoreScreen from './StoreScreen.svelte';
import type { StorePM } from './data';

afterEach(() => { resetBridgeForTests(); vi.unstubAllGlobals(); });

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
    const row = target.querySelector('.st-row.is-library')!;
    expect(row.textContent).toContain('Trust…');
    (row.querySelector('.st-row-open') as HTMLButtonElement).click();
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
    await vi.waitFor(() => expect(target.querySelector('.st-row-status')?.textContent).toContain('Trusted'));
    (target.querySelector('.st-row-more') as HTMLButtonElement).click();
    expect(document.body.textContent).toContain('Revoke Trust');
  } finally { await unmount(screen); target.remove(); }
});
