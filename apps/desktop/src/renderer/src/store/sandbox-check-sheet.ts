/* Opens the "Test in Sandbox…" sheet over the Store (via PM.modal), one at a
   time. The check runs in this window (kernel/sandbox-check.ts). */
import { flushSync, mount, unmount } from 'svelte';

import type { ModalHandle, ModalOptions } from '../overlays/types';
import type { StorePM } from './data';
import SandboxCheckSheet from './SandboxCheckSheet.svelte';
import { checkInSandbox } from './sandbox-check';

let open: { close(): void } | null = null;

export function openSandboxCheckSheet(PM: StorePM, localId: string, name: string): void {
  const modal: ((options: ModalOptions) => ModalHandle) | undefined = PM.modal;
  if (!modal) return;
  open?.close();
  const body = document.createElement('div');
  let component: ReturnType<typeof mount> | null = null;
  const handle = modal({
    body,
    width: 480,
    actions: [],
    onClose: () => {
      if (component) void unmount(component);
      component = null;
      if (open === current) open = null;
    }
  });
  handle.el.classList.add('account-modal', 'publish-modal');
  const current = { close: () => handle.close() };
  open = current;
  component = mount(SandboxCheckSheet, {
    target: body,
    props: { name, check: () => checkInSandbox(PM, localId), onclose: () => handle.close() }
  });
  flushSync();
}
