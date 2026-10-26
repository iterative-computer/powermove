import type { StorageBridge, StorageUpgradeProgress } from '../../../shared/storage-management';
import type { PMRegistry } from '../legacy/registry';
import { formatBytes } from '../transcription/format';
import { bridge } from '../kernel/bridge';

const KEY = 'storage-upgrade';
const stages = {
  recovery: 'Compressing recovery data', browser: 'Checking older media',
  sources: 'Verifying original files', cleanup: 'Removing verified duplicates and temporary files'
};

/** The ready renderer owns its origin's browser migration. Main leases the
 * job to one document, so another window can take over after a crash/reload. */
export function installStorageUpgradeNotice(PM: PMRegistry, host: StorageBridge | undefined = bridge()?.storage): () => void {
  if (!host?.upgrade || !PM.MediaStore?.optimizeLegacy) return () => {};
  let stopped = false, timer: ReturnType<typeof setTimeout> | undefined, pausedNotice = false, startedNotice = false;
  const progress = ({ stage, progress }: StorageUpgradeProgress) => {
    if (stopped) return;
    if (!startedNotice) {
      startedNotice = true;
      PM.toast('This update is optimizing storage in the background.', 6000, { key: 'storage-upgrade-start', kind: 'status', error: false });
    }
    PM.toast('Optimizing storage', 2200, { key: KEY, task: true, progress, detail: stages[stage], error: false });
  };
  const unsubscribe = host.onUpgradeProgress?.(progress);
  const retry = () => { if (!stopped) timer = setTimeout(() => { void run(); }, 60_000); };
  const run = async () => {
    try {
      if (stopped) return;
      let result = await host.upgrade!({ action: 'start' });
      if (stopped) return;
      if (result.state === 'busy') { retry(); return; }
      if (result.state === 'browser') {
        await PM.MediaStore.optimizeLegacy((done: number, total: number) => {
          progress({ stage: 'browser', progress: .25 + .25 * done / Math.max(1, total) });
        });
        if (stopped) return;
        result = await host.upgrade!({ action: 'browserComplete', token: result.token });
      }
      if (stopped) return;
      PM.dismissToast?.(KEY);
      PM.dismissToast?.('storage-upgrade-start');
      if (result.state === 'paused') throw new Error('Storage cleanup paused.');
      if (result.state === 'busy') { retry(); return; }
      if (result.state === 'complete' && result.notify) {
        const token = result.token;
        const acknowledge = () => { void host.upgrade!({ action: 'notified', token }).catch(() => {}); };
        PM.toast(result.freedBytes ? `Storage optimized · Freed about ${formatBytes(result.freedBytes)}` : 'Storage optimized', 10_000, {
          key: 'storage-upgrade-complete', kind: 'status', error: false, sticky: true, dismissible: true,
          action: { label: 'View Storage', run: () => { PM.SettingsUI?.open('storage'); acknowledge(); } },
          onDismiss: acknowledge,
        });
      }
    } catch {
      if (stopped) return;
      PM.dismissToast?.(KEY);
      if (!pausedNotice) {
        pausedNotice = true;
        PM.toast('Storage cleanup paused. It will resume automatically.', 6000, { key: 'storage-upgrade-paused', kind: 'status', error: false });
      }
      retry();
    }
  };
  const stop = () => {
    stopped = true;
    if (timer) clearTimeout(timer);
    unsubscribe?.();
    window.removeEventListener('beforeunload', stop);
  };
  window.addEventListener('beforeunload', stop, { once: true });
  // First paint and project recovery finish before background optimization.
  void Promise.resolve(PM.projectReady).then(() => {
    if (!stopped) timer = setTimeout(() => { void run(); }, 5000);
  }).catch(() => { retry(); });
  return stop;
}
