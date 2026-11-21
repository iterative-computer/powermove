import { flushSync, mount, unmount } from 'svelte';

import type { PMRegistry } from '../legacy/registry';
import Shell from './Shell.svelte';
import NativeReleaseBanner from './NativeReleaseBanner.svelte';

let mounted: Array<ReturnType<typeof mount>> = [];

export function installShell(PM: PMRegistry): void {
  const titlebar = window.document.getElementById('titlebar');
  const status = window.document.getElementById('status');
  if (!titlebar || !status) return;

  unmountShell();
  titlebar.replaceChildren();
  status.replaceChildren();
  titlebar.dataset.svelteShell = 'true';
  status.dataset.svelteShell = 'true';
  mounted = [
    mount(Shell, { target: titlebar, props: { PM, part: 'titlebar' } }),
    mount(Shell, { target: status, props: { PM, part: 'status' } })
  ];
  const banner = window.document.getElementById('native-release-banner');
  const launch = import.meta.env.VITE_POWERMOVE_NATIVE_LAUNCH_AT;
  if (banner && launch) {
    mounted.push(mount(NativeReleaseBanner, { target: banner, props: { launch } }));
  }
  flushSync();
}

/** Test/HMR seam. */
export function unmountShell(): void {
  for (const component of mounted) void unmount(component);
  mounted = [];
}
