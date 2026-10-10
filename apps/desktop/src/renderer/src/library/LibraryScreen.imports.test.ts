// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import type { LibraryItemDto } from '../../../shared/store-ipc';
import { installBridgeForTests, resetBridgeForTests } from '../kernel/bridge';
import LibraryScreen from './LibraryScreen.svelte';

afterEach(() => { resetBridgeForTests(); vi.unstubAllGlobals(); vi.restoreAllMocks(); });

const item = (id: string, category: 'panels' | 'effects' = 'panels'): LibraryItemDto => ({
  localId: id, name: id, description: '', version: '1.0.0', category, contributes: [category], vars: [], health: { state: 'ok' },
  enabled: true, trust: 'local', permissions: [], group: 'yours', maker: { you: true }, update: null, modified: false
});

it('shows only imported extensions and workspaces, including extensions without panels, and filters both sections', async () => {
  vi.stubGlobal('matchMedia', () => ({ matches: true, addListener() {}, removeListener() {} }));
  HTMLElement.prototype.scrollTo = vi.fn();
  let installed = [item('ae-timing'), item('ae-glow', 'effects'), item('unrelated-tool')];
  installBridgeForTests({ extensionStore: { library: async () => installed } } as any);
  const activate = vi.fn();
  const PM = {
    store: { get: () => ['ae-timing', 'ae-glow', 'removed-tool'] }, PANELS: {}, ICONS: new Proxy({}, { get: () => '<path/>' }),
    WS: { current: { id: 'original' }, activate, all: [
      { id: 'imported', name: 'Motion workspace', sourceApp: 'after-effects', layout: { docks: [] } },
      { id: 'unrelated', name: 'Unrelated workspace', layout: { docks: [] } }
    ] }, StoreUI: { open: vi.fn() }
  };
  const target = document.createElement('div'); document.body.append(target);
  const screen = mount(LibraryScreen, { target, props: { PM } });
  try {
    flushSync(() => screen.open());
    (target.querySelector('[aria-label="After Effects imports"]') as HTMLButtonElement).click(); flushSync();
    await vi.waitFor(() => expect(target.querySelectorAll('[data-extension-id]').length).toBe(2));
    const content = target.querySelector('.library-content')!;
    expect(content.textContent).toContain('ae-timing');
    expect(content.textContent).toContain('ae-glow');
    expect(content.textContent).toContain('Motion workspace');
    expect(content.textContent).not.toContain('unrelated-tool');
    expect(content.textContent).not.toContain('Unrelated workspace');
    expect(content.textContent).not.toContain('Not loaded');
    const search = target.querySelector('[aria-label="Search After Effects imports"]') as HTMLInputElement;
    search.value = 'Motion'; search.dispatchEvent(new Event('input')); flushSync();
    expect(content.querySelectorAll('[data-extension-id]').length).toBe(0);
    expect(content.textContent).toContain('Motion workspace');
    search.value = 'ae-glow'; search.dispatchEvent(new Event('input')); flushSync();
    expect(content.querySelectorAll('[data-extension-id]').length).toBe(1);
    expect(content.textContent).not.toContain('Motion workspace');
    search.value = ''; search.dispatchEvent(new Event('input')); flushSync();
    (content.querySelector('[aria-label="Activate Motion workspace"]') as HTMLButtonElement).click();
    expect(activate).toHaveBeenCalledWith('imported');
    installed = [item('ae-timing')];
    flushSync(() => screen.refresh(true));
    await vi.waitFor(() => expect(content.querySelectorAll('[data-extension-id]').length).toBe(1));
    expect(content.textContent).not.toContain('ae-glow');
  } finally { screen.close(); await unmount(screen); target.remove(); }
});

it('keeps the After Effects tab visible when empty and starts the workspace agent from it', async () => {
  vi.stubGlobal('matchMedia', () => ({ matches: true, addListener() {}, removeListener() {} }));
  HTMLElement.prototype.scrollTo = vi.fn();
  installBridgeForTests({ extensionStore: { library: async () => [] } } as any);
  const importWorkspace = vi.fn(async () => true);
  const PM = { store: { get: () => [] }, PANELS: {}, ICONS: new Proxy({}, { get: () => '<path/>' }), WS: { all: [] }, AgentUI: { importWorkspace } };
  const target = document.createElement('div'); document.body.append(target);
  const screen = mount(LibraryScreen, { target, props: { PM } });
  try {
    flushSync(() => screen.open('after-effects'));
    expect(target.querySelector('[aria-label="After Effects imports"]')).not.toBeNull();
    expect(target.textContent).toContain('Your After Effects imports will appear here.');
    (target.querySelector('.library-top-action button') as HTMLButtonElement).click();
    await vi.waitFor(() => expect(importWorkspace).toHaveBeenCalledWith('after-effects'));
    expect(screen.isOpen()).toBe(false);
  } finally { screen.close(); await unmount(screen); target.remove(); }
});
