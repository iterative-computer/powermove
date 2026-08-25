import { flushSync, mount, unmount } from 'svelte';

import type { PMRegistry } from '../legacy/registry';
import Shell from './Shell.svelte';

let mounted: Array<ReturnType<typeof mount>> = [];

export function wantsSvelteShell(PM: PMRegistry): boolean {
  const queryEnabled = new URLSearchParams(window.location.search).has('svelte-shell');
  return queryEnabled || PM.store?.get?.('shellSvelte', false) === true;
}

export function installShell(PM: PMRegistry): void {
  if (!wantsSvelteShell(PM)) return;

  const titlebar = window.document.getElementById('titlebar');
  const status = window.document.getElementById('status');
  if (!titlebar || !status) return;
  PM.SvelteShell = true;

  unmountShell();
  titlebar.replaceChildren();
  status.replaceChildren();
  titlebar.dataset.svelteShell = 'true';
  status.dataset.svelteShell = 'true';
  mounted = [
    mount(Shell, { target: titlebar, props: { PM, part: 'titlebar' } }),
    mount(Shell, { target: status, props: { PM, part: 'status' } })
  ];
  flushSync();
}

/** Test/HMR seam. The persisted switch remains untouched. */
export function unmountShell(): void {
  for (const component of mounted) void unmount(component);
  mounted = [];
}
