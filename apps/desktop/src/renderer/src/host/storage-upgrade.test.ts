import { afterEach, expect, it, vi } from 'vitest';
import { installStorageUpgradeNotice } from './storage-upgrade';
import type { StorageUpgradeRequest, StorageUpgradeResult } from '../../../shared/storage-management';

const stops: Array<() => void> = [];
afterEach(() => { for (const stop of stops.splice(0)) stop(); vi.useRealTimers(); vi.unstubAllGlobals(); });
function fixture() {
  vi.useFakeTimers(); vi.stubGlobal('window', { addEventListener: vi.fn(), removeEventListener: vi.fn() });
  const PM: any = { projectReady: Promise.resolve(), MediaStore: { optimizeLegacy: vi.fn(async () => {}) }, toast: vi.fn(), dismissToast: vi.fn(), SettingsUI: { open: vi.fn() } };
  const host = { upgrade: vi.fn(async (_request: StorageUpgradeRequest): Promise<StorageUpgradeResult> => ({ state: 'complete', notify: false })),
    onUpgradeProgress: vi.fn((_listener: any) => vi.fn()) };
  return { PM, host, start: () => { const stop = installStorageUpgradeNotice(PM, host as any); stops.push(stop); return stop; } };
}

it('starts after readiness without blocking boot, completes browser work before reporting it, and notifies with savings', async () => {
  const { PM, host, start } = fixture(), order: string[] = [];
  host.upgrade.mockImplementation(async request => {
    if (request.action === 'start') {
      host.onUpgradeProgress.mock.calls[0]![0]({ stage: 'recovery', progress: 0 });
      return { state: 'browser', token: 'session' };
    }
    if (request.action === 'browserComplete') { order.push('finished'); return { state: 'complete', notify: true, token: 'session', freedBytes: 2_000_000 }; }
    return { state: 'complete', notify: false };
  });
  PM.MediaStore.optimizeLegacy.mockImplementation(async (progress: any) => { order.push('browser'); progress(1, 1); });
  start(); expect(host.upgrade).not.toHaveBeenCalled();
  await vi.advanceTimersByTimeAsync(5000);
  expect(order).toEqual(['browser', 'finished']);
  expect(PM.toast).toHaveBeenCalledWith('This update is optimizing storage in the background.', expect.any(Number), expect.objectContaining({ error: false }));
  const notice = PM.toast.mock.calls.find(([message]: any[]) => message === 'Storage optimized · Freed about 2 MB');
  expect(notice?.[2]).toMatchObject({ sticky: true, dismissible: true, kind: 'status', action: { label: 'View Storage' } });
  expect(host.upgrade.mock.calls.map(([request]) => request.action)).toEqual(['start', 'browserComplete']);
  notice![2].onDismiss(); await vi.advanceTimersByTimeAsync(0);
  expect(host.upgrade).toHaveBeenLastCalledWith({ action: 'notified', token: 'session' });
});

it('quietly skips an acknowledged upgrade on later launches', async () => {
  const { PM, host, start } = fixture(); start(); await vi.advanceTimersByTimeAsync(5000);
  expect(host.upgrade).toHaveBeenCalledOnce(); expect(PM.MediaStore.optimizeLegacy).not.toHaveBeenCalled(); expect(PM.toast).not.toHaveBeenCalled();
});

it('shows a paused notice once and retries browser failure without reporting completion', async () => {
  const { PM, host, start } = fixture(); host.upgrade.mockResolvedValue({ state: 'browser', token: 'session' });
  PM.MediaStore.optimizeLegacy.mockRejectedValue(new Error('Browser transaction failed'));
  start(); await vi.advanceTimersByTimeAsync(5000); await vi.advanceTimersByTimeAsync(60_000);
  expect(host.upgrade).toHaveBeenCalledTimes(2);
  expect(PM.toast.mock.calls.filter(([message]: any[]) => message === 'Storage cleanup paused. It will resume automatically.')).toHaveLength(1);
  expect(host.upgrade.mock.calls.some(([request]) => request.action === 'browserComplete')).toBe(false);
});

it('lets another window finish the shared job and retries a busy response', async () => {
  const { PM, host, start } = fixture(); host.upgrade.mockResolvedValueOnce({ state: 'busy' });
  start(); await vi.advanceTimersByTimeAsync(5000); expect(PM.toast).not.toHaveBeenCalled();
  await vi.advanceTimersByTimeAsync(60_000); expect(host.upgrade).toHaveBeenCalledTimes(2);
});

it('stops timers and listeners on navigation and does not finish an old browser migration', async () => {
  const { PM, host, start } = fixture(); let release!: () => void;
  host.upgrade.mockResolvedValue({ state: 'browser', token: 'session' });
  PM.MediaStore.optimizeLegacy.mockImplementation(() => new Promise<void>(resolve => { release = resolve; }));
  const stop = start(); await vi.advanceTimersByTimeAsync(5000); stop(); release(); await vi.advanceTimersByTimeAsync(60_000);
  expect(host.upgrade).toHaveBeenCalledOnce();
  expect(window.removeEventListener).toHaveBeenCalledWith('beforeunload', expect.any(Function));
});
