/* The home owns the sidebar and search field; this mounts the live Store in
   its content pane while retaining the PM.StoreUI entry point. */
import { mount, unmount } from 'svelte';

import type { PMRegistry } from '../legacy/registry';
import type { StorePage, StoreUISurface } from './data';
import StoreScreen from './StoreScreen.svelte';

export function install(PM: PMRegistry): void {
  type Screen = { open(page?: StorePage): void; close(): void; isOpen(): boolean; setActive(on: boolean): void; setSearch(text: string): void };
  let screen: Screen | null = null;
  let host: HTMLElement | null = null;
  function ensure(target: HTMLElement): Screen {
    if (screen && host !== target) void unmount(screen);
    if (!screen || host !== target) screen = mount(StoreScreen, { target, props: { PM } });
    host = target;
    return screen;
  }
  const surface: StoreUISurface & { attach(target: HTMLElement): void; setActive(on: boolean): void; setSearch(text: string): void } = {
    attach: (target) => { ensure(target); },
    setActive: (on) => screen?.setActive(on),
    setSearch: (text) => screen?.setSearch(text),
    open: (page?: StorePage) => {
      PM.ProjectsScreen?.show('store');
      screen?.open(page);
    },
    close: () => screen?.close(),
    get isOpen() { return !!PM.ProjectsScreen?.isOpen && PM.ProjectsScreen?.section === 'store' && (screen?.isOpen() ?? false); }
  };
  PM.StoreUI = surface;
}
