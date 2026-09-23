import type { ExtensionRecord } from '../../../shared/extensions';
import type { PowermoveAPI } from '../kernel/api';
import type { PMRegistry } from '../legacy/registry';
import { bridge } from '../kernel/bridge';

export type ExtensionSnapshot = Record<string, { version: string | null; enabled: boolean; health: string }>;
export type ExtensionUpdateNotice = {
  key: string;
  id: string;
  name: string;
  kind: 'compatibility' | 'included';
  message: string;
};
type SavedState = { version: string; extensions: ExtensionSnapshot; pending: ExtensionUpdateNotice[]; dismissed: string[] };
const STORAGE_KEY = 'state';
const failed = new Set(['build-error', 'manifest-error', 'activation-error', 'runtime-error', 'needs-update']);

export function extensionSnapshot(records: readonly ExtensionRecord[]): ExtensionSnapshot {
  return Object.fromEntries(records.filter(record => record.scope !== 'builtin').map(record => [record.id, {
    version: record.manifest?.version ?? null, enabled: record.enabled, health: record.health.state,
  }]));
}

/** Only a release's explicit declarations can establish feature equivalence.
 * A shared name or contribution kind (such as "panels") is not evidence. */
export function extensionUpdateNotices(records: readonly ExtensionRecord[], version: string,
  previous?: Pick<SavedState, 'version' | 'extensions'>): ExtensionUpdateNotice[] {
  const notices: ExtensionUpdateNotice[] = [];
  const builtins = records.filter(record => record.scope === 'builtin' && record.manifest);
  for (const record of records) {
    if (record.scope === 'builtin') continue;
    const name = record.manifest?.name ?? record.id;
    const before = previous?.extensions[record.id];
    if (previous && previous.version !== version && before?.enabled && before.health === 'ok'
      && failed.has(record.health.state)
      && (record.manifest === null || before.version === record.manifest.version)) {
      notices.push({ key: `compatibility:${version}:${record.id}`, id: record.id, name, kind: 'compatibility',
        message: `${name} reported a problem after updating to Powermove ${version}. Review the extension before using it.` });
    }
    const features = record.manifest?.features ?? [];
    const direct = builtins.filter(builtin => builtin.manifest!.integrates?.includes(record.id));
    const providers = builtins.filter(builtin => builtin.manifest!.features?.some(feature => features.includes(feature)));
    const covered = new Set(providers.flatMap(builtin => builtin.manifest!.features ?? []));
    const complete = features.length > 0 && features.every(feature => covered.has(feature));
    const included = direct.length ? direct : complete ? providers : [];
    if (included.length) {
      const identities = included.map(builtin => builtin.id).sort().join(',');
      notices.push({ key: `included:${record.id}:${record.manifest?.version ?? 'unknown'}:${identities}:${[...features].sort().join(',')}`,
        id: record.id, name, kind: 'included',
        message: `Powermove now includes ${name}'s ${direct.length ? 'functionality' : 'declared features'} in ${included.map(builtin => builtin.manifest!.name).join(', ')}. Review before deciding whether to keep the extension.` });
    }
  }
  return notices;
}

/** Persist across restarts; never modify, disable, or repair an extension on the user's behalf. */
export function installExtensionUpdateNotices(PM: PMRegistry): () => void {
  const api = PM.Kernel?.api?.('extension-update-notices') as PowermoveAPI | undefined;
  const updates = bridge()?.updates;
  if (!api || !updates) return () => {};
  const previous = api.storage.get<SavedState>(STORAGE_KEY);
  const dismissed = new Set(previous?.dismissed ?? []);
  let pending = new Map((previous?.pending ?? []).map(notice => [notice.key, notice]));
  const shown = new Set<string>();
  let current: string | null = null;
  let ready = false;
  let alive = true;
  const toastKey = (key: string) => `extension-update:${key}`;
  const persist = (records = api.extensions.list()) => {
    if (current) api.storage.set(STORAGE_KEY, { version: current, extensions: extensionSnapshot(records),
      pending: [...pending.values()], dismissed: [...dismissed].slice(-256) } satisfies SavedState);
  };
  const dismiss = (key: string) => {
    dismissed.add(key);
    pending.delete(key);
    shown.delete(key);
    PM.dismissToast?.(toastKey(key));
    persist();
  };
  const refresh = () => {
    if (!alive || !ready || !current) return;
    const records = api.extensions.list();
    const byId = new Map(records.map(record => [record.id, record]));
    for (const [key, notice] of pending) {
      const record = byId.get(notice.id);
      if (!record || (notice.kind === 'compatibility' && !failed.has(record.health.state))) {
        pending.delete(key);
        if (shown.delete(key)) PM.dismissToast?.(toastKey(key));
      }
    }
    for (const notice of extensionUpdateNotices(records, current, previous ?? undefined)) {
      if (!dismissed.has(notice.key)) pending.set(notice.key, notice);
    }
    persist(records);
    for (const notice of pending.values()) {
      if (shown.has(notice.key) || dismissed.has(notice.key)) continue;
      shown.add(notice.key);
      PM.toast(notice.message, 6000, {
        key: toastKey(notice.key), corner: 'top-right', sticky: true, dismissible: true,
        kind: notice.kind === 'compatibility' ? 'alert' : 'status',
        source: { id: notice.id, name: notice.name }, icon: notice.kind === 'compatibility' ? 'caution' : 'puzzle',
        action: { label: 'Review extensions', run: () => { dismiss(notice.key); PM.SettingsUI?.open?.('extensions'); } },
        onDismiss: () => dismiss(notice.key),
      });
    }
  };
  const subscriptions = [
    api.events.on('extensions:ready', () => { ready = true; refresh(); }),
    api.events.on('extensions:changed', refresh),
    api.events.on('extension:loaded', refresh),
  ];
  void updates.status().then(state => { if (alive) { current = state.current; refresh(); } }).catch(() => undefined);
  return () => {
    alive = false;
    for (const subscription of subscriptions) subscription.dispose();
    for (const key of shown) PM.dismissToast?.(toastKey(key));
    pending.clear();
  };
}
