import { flushSync, mount, unmount } from 'svelte';

import Modal from './Modal.svelte';
import { inertOverlayHosts, restoreOverlayHosts, type InertSnapshot } from './inert';
import type { ModalHandle, ModalOptions, OverlayPM } from './types';

type ModalInstance = ReturnType<typeof mount> & {
  element(): HTMLElement;
  bodyElement(): HTMLElement;
  focusInitial(): void;
  setPending(value: boolean): void;
  setError(value: unknown): void;
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
    dismiss: () => void;
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
    let pending = false;
    const dismiss = () => {
      if (options.dismissible !== false && !pending) handle.close();
    };
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
          if (pending) return;
          const action = options.actions?.[index];
          instance.setError(null);
          const failed = (error: unknown): void => {
            if (handle.el.isConnected) {
              instance.setError(error instanceof Error ? error.message : String(error));
              flushSync();
            }
          };
          try {
            const result = action?.run?.();
            if (result && typeof (result as PromiseLike<unknown>).then === 'function') {
              pending = true;
              instance.setPending(true);
              flushSync();
              void Promise.resolve(result).then(value => {
                if (value !== false) handle.close();
              }, failed).finally(() => {
                pending = false;
                instance.setPending(false);
                flushSync();
              });
            } else if (result !== false) handle.close();
          } catch (error) { failed(error); }
        },
        onclose: dismiss
      }
    }) as ModalInstance;
    flushSync();
    handle = {
      el: instance.element(),
      body: instance.bodyElement(),
      close: () => this.close(handle)
    };
    this.stack.push({ instance, handle, trigger, dismiss, onClose: options.onClose });
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
    scrim.onclick = latest ? latest.dismiss : null;
  }
}
