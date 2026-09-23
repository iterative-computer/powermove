/* The Store is a full-screen surface like Settings and the Projects home. This
   installer keeps a PM.StoreUI contract and mounts the Svelte screen on first
   open. The screen reads the registry and this Mac's Library through the
   `extensionStore` bridge. */
import { mount } from 'svelte';

import type { PMRegistry } from '../legacy/registry';
import type { StorePage, StoreUISurface } from './data';
import StoreScreen from './StoreScreen.svelte';

export function install(PM: PMRegistry): void {
  let screen: ReturnType<typeof mount<{ PM: PMRegistry }, { open(page?: StorePage): void; close(): void; isOpen(): boolean }>> | null = null;
  function ensure() {
    screen ??= mount(StoreScreen, { target: document.body, props: { PM } });
    return screen;
  }
  const surface: StoreUISurface = {
    open: (page?: StorePage) => ensure().open(page),
    close: () => screen?.close(),
    get isOpen() { return screen?.isOpen() ?? false; }
  };
  PM.StoreUI = surface;
}
