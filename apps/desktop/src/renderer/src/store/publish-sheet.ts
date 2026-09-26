/* Opens the publish sheet over the Store (via PM.modal), one at a time. The
   plan comes from main (`store:publish-prepare`); the sheet only fills in the
   form, and main shows the native confirmation. */
import { flushSync, mount, unmount } from 'svelte';

import type { PublishPlanDto, StorePublishResult } from '../../../shared/publish';
import type { StoreBridge } from '../../../shared/store-ipc';
import type { ModalHandle, ModalOptions } from '../overlays/types';
import type { StorePM } from './data';
import PublishSheet from './PublishSheet.svelte';
import { checkInSandbox } from './sandbox-check';

let open: { localId: string; close(): void } | null = null;

export function openPublishSheet(
  PM: StorePM,
  bridge: StoreBridge,
  plan: PublishPlanDto,
  onpublished: (result: Extract<StorePublishResult, { published: true }>) => void
): void {
  const modal: ((options: ModalOptions) => ModalHandle) | undefined = PM.modal;
  if (!modal) return;
  open?.close();
  const body = document.createElement('div');
  let component: ReturnType<typeof mount> | null = null;
  const handle = modal({
    body,
    width: 520,
    actions: [],
    onClose: () => {
      if (component) void unmount(component);
      component = null;
      if (open === current) open = null;
    }
  });
  handle.el.classList.add('account-modal', 'publish-modal');
  const current = { localId: plan.localId, close: () => handle.close() };
  open = current;
  component = mount(PublishSheet, {
    target: body,
    props: { plan, bridge, check: () => checkInSandbox(PM, plan.localId), onclose: () => handle.close(), onpublished }
  });
  flushSync();
}
