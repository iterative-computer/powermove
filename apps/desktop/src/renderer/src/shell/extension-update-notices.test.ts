import { installBridgeForTests, resetBridgeForTests } from '../kernel/bridge';
import { afterEach, expect, it, vi } from 'vitest';
import type { ExtensionRecord } from '../../../shared/extensions';
import { parseManifest } from '../../../shared/extensions';
import { extensionSnapshot, extensionUpdateNotices, installExtensionUpdateNotices } from './extension-update-notices';

function record(id: string, patch: Partial<ExtensionRecord> = {}): ExtensionRecord {
  return { id, scope: 'user', enabled: true, health: { state: 'ok' }, dir: '/extensions/' + id,
    bundleUrl: null, bundleHash: null, updatedAt: 0,
    manifest: { id, name: id, version: '1.0.0', apiVersion: 1 }, ...patch };
}
const previous = { version: '1.0.0', extensions: extensionSnapshot([record('custom')]) };

afterEach(() => vi.unstubAllGlobals());

it('reports new failures after an app update, including extensions quarantined by a runtime failure', () => {
  const broken = record('custom', { enabled: false, health: { state: 'runtime-error', error: 'Missing method' } });
  expect(extensionUpdateNotices([broken], '2.0.0', previous)).toEqual([expect.objectContaining({ kind: 'compatibility', id: 'custom' })]);
  expect(extensionUpdateNotices([broken], '1.0.0', previous)).toEqual([]);
  expect(extensionUpdateNotices([broken], '2.0.0')).toEqual([]);
  expect(extensionUpdateNotices([record('custom', { enabled: false, health: { state: 'disabled' } })], '2.0.0', previous)).toEqual([]);
  expect(extensionUpdateNotices([broken], '2.0.0', { ...previous, extensions: extensionSnapshot([broken]) })).toEqual([]);
});

it('does not blame the app update for a newly installed or separately updated extension', () => {
  const broken = record('new-extension', { health: { state: 'build-error', error: 'Syntax' } });
  expect(extensionUpdateNotices([broken], '2.0.0', previous)).toEqual([]);
  const changed = record('custom');
  changed.manifest!.version = '2.0.0'; changed.health = broken.health;
  expect(extensionUpdateNotices([changed], '2.0.0', previous)).toEqual([]);
});

it('requires explicit shipped metadata or complete precise feature coverage to announce inclusion', () => {
  const custom = record('custom'); custom.manifest!.features = ['frame-grid', 'frame-snap'];
  const builtin = record('timeline', { scope: 'builtin' }); builtin.manifest!.features = ['frame-grid'];
  expect(extensionUpdateNotices([custom, builtin], '2.0.0')).toEqual([]);
  builtin.manifest!.features.push('frame-snap');
  expect(extensionUpdateNotices([custom, builtin], '2.0.0')).toEqual([expect.objectContaining({ kind: 'included', id: 'custom' })]);
  delete custom.manifest!.features;
  expect(extensionUpdateNotices([custom, builtin], '2.0.0')).toEqual([]);
  builtin.manifest!.integrates = ['custom'];
  expect(extensionUpdateNotices([custom, builtin], '2.0.0')).toHaveLength(1);
  builtin.scope = 'user';
  expect(extensionUpdateNotices([custom, builtin], '2.0.0')).toEqual([]);
});

it('validates and preserves additive feature declarations', () => {
  const manifest = { id: 'custom', name: 'Custom', apiVersion: 1, version: '1.0.0', features: ['frame-snap', 'frame-snap'], integrates: ['old-snap'] };
  expect(parseManifest(manifest)).toMatchObject({ ok: true, manifest: { features: ['frame-snap'], integrates: ['old-snap'] } });
  expect(parseManifest({ ...manifest, features: ['not a stable identifier'] }).ok).toBe(false);
  expect(parseManifest({ ...manifest, integrates: [7] }).ok).toBe(false);
});

it('waits for extension boot, catches late health errors, and remembers review/dismissal without changing extensions', async () => {
  const storage = new Map<string, any>([['state', { ...previous, pending: [], dismissed: [] }]]);
  const handlers = new Map<string, () => void>();
  let records = [record('custom')];
  const toast = vi.fn(), open = vi.fn(), setEnabled = vi.fn();
  vi.stubGlobal('window', { powermove: { updates: { status: async () => ({ current: '2.0.0' }) } } });
  installBridgeForTests((window as any).powermove);
  const api = { storage: { get: (key: string) => storage.get(key), set: (key: string, value: unknown) => storage.set(key, value) },
    events: { on: (event: string, callback: () => void) => { handlers.set(event, callback); return { dispose: () => handlers.delete(event) }; } },
    extensions: { list: () => records, setEnabled } };
  const PM = { Kernel: { api: () => api }, toast, dismissToast: vi.fn(), SettingsUI: { open } };
  let dispose = installExtensionUpdateNotices(PM as any);
  await Promise.resolve();
  expect(storage.get('state').version).toBe('1.0.0');
  handlers.get('extensions:ready')!();
  expect(toast).not.toHaveBeenCalled();
  records = [record('custom', { health: { state: 'activation-error', error: 'Missing method' } })];
  handlers.get('extensions:changed')!();
  expect(toast).toHaveBeenCalledOnce();
  expect(toast.mock.calls[0]![2]).toMatchObject({ sticky: true, kind: 'alert', corner: 'top-right' });
  handlers.get('extensions:changed')!();
  expect(toast).toHaveBeenCalledOnce();
  dispose();
  dispose = installExtensionUpdateNotices(PM as any);
  await Promise.resolve(); handlers.get('extensions:ready')!();
  expect(toast).toHaveBeenCalledTimes(2);
  toast.mock.calls.at(-1)![2].action.run();
  expect(open).toHaveBeenCalledWith('extensions');
  expect(setEnabled).not.toHaveBeenCalled();
  dispose();
  dispose = installExtensionUpdateNotices(PM as any);
  await Promise.resolve(); handlers.get('extensions:ready')!();
  expect(toast).toHaveBeenCalledTimes(2);
  dispose();
});
