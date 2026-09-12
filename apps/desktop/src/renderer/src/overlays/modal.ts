import { flushSync, mount, unmount } from 'svelte';

import Modal from './Modal.svelte';
import { inertOverlayHosts, restoreOverlayHosts, type InertSnapshot } from './inert';
import type { ModalHandle, ModalOptions, OverlayPM } from './types';

type ModalInstance = ReturnType<typeof mount> & {
  element(): HTMLElement;
  bodyElement(): HTMLElement;
  focusInitial(): void;
};

let modalId = 0;

function activeTrigger(): HTMLElement | null {
  return document.activeElement instanceof HTMLElement && document.activeElement !== document.body
    ? document.activeElement
    : null;
}

export class ModalController {
  private readonly stack: Array<{
    instance: ModalInstance;
    handle: ModalHandle;
    trigger: HTMLElement | null;
    onClose?: () => void;
  }> = [];
  private inertSnapshot: InertSnapshot = [];

  constructor(private readonly PM: OverlayPM) {}

  open(options: ModalOptions): ModalHandle {
    this.PM.closeMenus?.();
    const scrim = document.getElementById('scrim');
    if (!scrim) throw new Error('PM.modal requires #scrim');
    const trigger = activeTrigger();
    if (!this.stack.length) {
      this.inertSnapshot = inertOverlayHosts();
    }
    scrim.classList.add('on');
    const titleId = `pm-modal-title-${++modalId}`;
    let handle!: ModalHandle;
    const instance = mount(Modal, {
      target: document.body,
      props: {
        title: options.title,
        bodyContent: options.body,
        actions: options.actions ?? [],
        width: options.width ?? 460,
        fill: options.fill ?? false,
        titleId,
        onaction: (index: number) => {
          const action = options.actions?.[index];
          if (action?.run?.() === false) return;
          handle.close();
        },
        onclose: () => handle.close()
      }
    }) as ModalInstance;
    flushSync();
    handle = {
      el: instance.element(),
      body: instance.bodyElement(),
      close: () => this.close(handle)
    };
    this.stack.push({ instance, handle, trigger, onClose: options.onClose });
    this.syncScrim();
    queueMicrotask(() => {
      if (this.stack.some((entry) => entry.handle === handle)) instance.focusInitial();
    });
    return handle;
  }

  close(handle: ModalHandle): void {
    const index = this.stack.findIndex((entry) => entry.handle === handle);
    if (index < 0) return;
    const [entry] = this.stack.splice(index, 1);
    if (!entry) return;
    void unmount(entry.instance);
    entry.handle.el.remove();
    if (!this.stack.length) {
      restoreOverlayHosts(this.inertSnapshot);
      this.inertSnapshot = [];
    }
    this.syncScrim();
    entry.onClose?.();
    if (entry.trigger?.isConnected && !entry.trigger.closest('[inert]')) {
      entry.trigger.focus({ preventScroll: true });
    } else if (this.stack.length) {
      this.stack[this.stack.length - 1]?.instance.focusInitial();
    } else {
      const app = document.getElementById('app');
      if (app) {
        app.tabIndex = -1;
        app.focus({ preventScroll: true });
      }
    }
  }

  closeAll(): void {
    for (const entry of [...this.stack].reverse()) entry.handle.close();
  }

  private syncScrim(): void {
    const scrim = document.getElementById('scrim');
    if (!scrim) return;
    scrim.classList.toggle('on', this.stack.length > 0);
    const latest = this.stack[this.stack.length - 1];
    scrim.onclick = latest ? () => latest.handle.close() : null;
  }
}
