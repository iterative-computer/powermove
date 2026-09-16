import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { AppUpdateState } from '../../../shared/ipc';
import { appUpdateState, dismissToast, installAppUpdates, subscribeAppUpdates } from './app-updates';

type Bus = { handlers: Map<string, () => void>; on(event: string, fn: () => void): () => void; emit(event: string): void };

function harness(initial: AppUpdateState) {
  const storage = new Map<string, unknown>();
  const toast = vi.fn();
  const dismissToastKey = vi.fn();
  const install = vi.fn().mockResolvedValue(undefined);
  let changed: ((state: AppUpdateState) => void) | null = null;
  const bus: Bus = {
    handlers: new Map(),
    on(event, fn) { this.handlers.set(event, fn); return () => this.handlers.delete(event); },
    emit(event) { this.handlers.get(event)?.(); }
  };
  let homeOpen = false;
  (globalThis as { window: unknown }).window = {
    powermove: {
      updates: {
        status: () => Promise.resolve(initial),
        install,
        check: () => Promise.resolve(),
        onChanged: (cb: (state: AppUpdateState) => void) => { changed = cb; return () => { changed = null; }; }
      }
    }
  };
  const PM = {
    Kernel: { api: () => ({ ui: { toast }, storage: { get: (k: string) => storage.get(k), set: (k: string, v: unknown) => storage.set(k, v) } }) },
    dismissToast: dismissToastKey,
    bus,
    ProjectsScreen: { get isOpen() { return homeOpen; } }
  };
  const dispose = installAppUpdates(PM as never);
  return { toast, dismissToastKey, install, dispose, push: (s: AppUpdateState) => changed?.(s), setHome(open: boolean) { homeOpen = open; bus.emit('projects:screen'); } };
}

const ready: AppUpdateState = { status: 'ready', current: '1.0.0', version: '1.0.1' };

describe('app update notice', () => {
  let dispose = () => {};
  beforeEach(() => { vi.clearAllMocks(); });
  afterEach(() => { dispose(); });

  it('shows a sticky bottom-right toast over the editor once an update is staged', async () => {
    const h = harness({ status: 'idle', current: '1.0.0', version: null });
    dispose = h.dispose;
    await Promise.resolve();
    expect(h.toast).not.toHaveBeenCalled();
    h.push(ready);
    expect(h.toast).toHaveBeenCalledWith('Powermove 1.0.1 is ready to install', expect.objectContaining({ sticky: true, corner: 'bottom-right', key: 'app-update' }));
    expect(appUpdateState()).toEqual(ready);
    h.toast.mock.calls[0]![1].action.run();
    expect(h.install).toHaveBeenCalledTimes(1);
  });

  it('keeps the toast off the home screen and remembers a dismissal per version', () => {
    const h = harness(ready);
    dispose = h.dispose;
    h.setHome(true);
    h.push(ready);
    expect(h.toast).not.toHaveBeenCalled();
    h.setHome(false);
    expect(h.toast).toHaveBeenCalledTimes(1);
    dismissToast();
    expect(h.dismissToastKey).toHaveBeenCalledWith('app-update');
    h.setHome(true); h.setHome(false);
    expect(h.toast).toHaveBeenCalledTimes(1);
    h.push({ ...ready, version: '1.0.2' });
    expect(h.toast).toHaveBeenCalledTimes(2);
  });

  it('feeds the home sidebar block regardless of dismissal', () => {
    const h = harness(ready);
    dispose = h.dispose;
    const seen: Array<AppUpdateState | null> = [];
    const off = subscribeAppUpdates((s) => seen.push(s));
    h.push(ready);
    expect(seen.at(-1)).toEqual(ready);
    off();
  });
});
