/* The Store is a full-screen surface like Settings and the Projects home. This
   installer keeps a PM.StoreUI contract and mounts the Svelte screen on first
   open. Design pass only: the screen reads fixture data, not a registry. */
import { mount } from 'svelte';

import type { PMRegistry } from '../legacy/registry';
import StoreScreen, { type StorePage } from './StoreScreen.svelte';

export function install(PM: PMRegistry): void {
  let screen: any = null;
  function ensure() {
    if (!screen) screen = mount(StoreScreen, { target: document.body, props: { PM } });
    return screen;
  }
  (PM as any).StoreUI = {
    open: (page?: StorePage) => ensure().open(page),
    close: () => screen?.close(),
    get isOpen() { return !!screen?.isOpen(); }
  };
}
