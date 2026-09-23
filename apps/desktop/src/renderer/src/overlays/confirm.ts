import type { PMRegistry } from '../legacy/registry';
import type { ConfirmRequest } from '../../../shared/ipc';
import { bridge } from '../kernel/bridge';

export type ConfirmOptions = ConfirmRequest;

/**
 * Plain yes-or-no prompts are native sheets. The Svelte modal is only the
 * fallback for hosts without the Electron bridge (web preview, unit tests).
 */
export async function confirm(PM: PMRegistry, options: ConfirmOptions): Promise<boolean> {
  const host = bridge();
  if (typeof host?.confirm === 'function') {
    try {
      return !!(await host.confirm(options));
    } catch (error) {
      console.error('[confirm] native sheet failed, falling back to modal', error);
    }
  }
  return new Promise<boolean>((resolve) => {
    let settled = false;
    const done = (value: boolean): void => {
      if (settled) return;
      settled = true;
      resolve(value);
    };
    const handle = PM.modal?.({
      title: options.message,
      body: options.detail ?? '',
      width: 420,
      actions: [
        { label: 'Cancel', run: () => done(false) },
        { label: options.confirmLabel ?? 'OK', pri: true, run: () => done(true) }
      ],
      onClose: () => done(false)
    });
    if (!handle) done(false);
  });
}
