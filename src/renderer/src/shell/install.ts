import { flushSync, mount, unmount } from 'svelte';

import type { PMRegistry } from '../legacy/registry';
import Shell from './Shell.svelte';

let mounted: Array<ReturnType<typeof mount>> = [];

/* Phase 5.4: the Svelte chrome is the DEFAULT. The legacy engines remain one
   release behind an escape hatch (?legacy-shell or the shellLegacy store key)
   and are deleted in Phase 6. */
export function wantsSvelteShell(PM: PMRegistry): boolean {
  const params = new URLSearchParams(window.location.search);
  if (params.has('svelte-shell')) return true; // explicit opt-in always wins
  if (params.has('legacy-shell')) return false;
  if (PM.store?.get?.('shellLegacy', false) === true) return false;
  return true;
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
