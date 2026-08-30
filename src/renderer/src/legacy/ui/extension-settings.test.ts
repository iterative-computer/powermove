// @vitest-environment happy-dom
import { describe, expect, it, vi } from 'vitest';

import type { ExtensionRecord, ExtensionsBridge, ExtensionsChangedEvent } from '../../../../shared/extensions';
import { createExtensionSettingsControl, extensionSettingsLabels } from './extension-settings';

function record(id: string, overrides: Partial<ExtensionRecord> = {}): ExtensionRecord {
  return {
    id,
    scope: 'user',
    manifest: { id, name: id, version: '1.0.0', apiVersion: 1, author: 'user' },
    dir: `/extensions/${id}`,
    enabled: true,
    bundleUrl: `app://powermove/ext/${id}/bundle.js`,
    bundleHash: 'hash',
    health: { state: 'ok' },
    updatedAt: 1,
    ...overrides
  };
}

function bridge(initial: ExtensionRecord[]) {
  let records = initial;
  let listener: ((event: ExtensionsChangedEvent) => void) | undefined;
  const stop = vi.fn();
  const api = {
    list: vi.fn(async () => records),
    setEnabled: vi.fn(async ({ id, enabled }) => {
      records = records.map((item) => item.id === id
        ? { ...item, enabled, health: enabled ? { state: 'ok' as const } : { state: 'disabled' as const } }
        : item);
      return records;
    }),
    remove: vi.fn(async () => records),
    reload: vi.fn(async () => records),
    create: vi.fn(async () => records),
    reveal: vi.fn(async () => undefined),
    readSource: vi.fn(async () => []),
    reportHealth: vi.fn(),
    onChanged: vi.fn((next) => { listener = next; return stop; })
  } satisfies ExtensionsBridge;
  return { api, stop, emit: (event: ExtensionsChangedEvent) => listener?.(event), setRecords: (next: ExtensionRecord[]) => { records = next; } };
}

describe('extension settings control', () => {
  it('shows agent, user, built-in, disabled, replaced, and malformed records', async () => {
    const harness = bridge([
      record('builtin-ext', { scope: 'builtin', manifest: { id: 'builtin-ext', name: 'Builtin', version: '1.0.0', apiVersion: 1, author: 'powermove' } }),
      record('agent-ext', { manifest: { id: 'agent-ext', name: 'Agent tool', version: '2.0.0', apiVersion: 1, author: 'agent' } }),
      record('disabled-ext', { enabled: false, health: { state: 'disabled' } }),
      record('old-timeline', { health: { state: 'replaced', by: 'new-timeline' } }),
      record('broken-ext', { manifest: null, bundleUrl: null, bundleHash: null, health: { state: 'manifest-error', error: 'Invalid manifest' } })
    ]);
    const control = createExtensionSettingsControl(harness.api);

    await vi.waitFor(() => expect(control.element.querySelectorAll('[data-extension-id]')).toHaveLength(5));
    expect(control.element.textContent).toContain('5 extensions discovered');
    expect(control.element.querySelector('[data-extension-id="agent-ext"]')?.textContent).toContain('Agent');
    expect(control.element.querySelector('[data-extension-id="builtin-ext"]')?.textContent).toContain('Built in');
    expect(control.element.querySelector('[data-extension-id="disabled-ext"]')?.textContent).toContain('Off');
    expect(control.element.querySelector('[data-extension-id="old-timeline"]')?.textContent).toContain('Replaced by new-timeline');
    expect(control.element.querySelector('[data-extension-id="broken-ext"]')?.textContent).toContain('Invalid manifest');

    control.destroy();
    expect(harness.stop).toHaveBeenCalledOnce();
  });

  it('updates while Settings is open and toggles through the native bridge', async () => {
    const harness = bridge([record('one')]);
    const control = createExtensionSettingsControl(harness.api);
    await vi.waitFor(() => expect(control.element.querySelectorAll('[data-extension-id]')).toHaveLength(1));

    harness.setRecords([record('one'), record('new-agent', {
      manifest: { id: 'new-agent', name: 'New agent extension', version: '1.0.0', apiVersion: 1, author: 'agent' }
    })]);
    harness.emit({ ids: ['new-agent'], reason: 'create' });
    await vi.waitFor(() => expect(control.element.querySelectorAll('[data-extension-id]')).toHaveLength(2));

    const toggle = control.element.querySelector<HTMLInputElement>('[data-extension-id="new-agent"] input');
    if (!toggle) throw new Error('New agent extension toggle was not rendered.');
    toggle.checked = false;
    toggle.dispatchEvent(new Event('change'));
    await vi.waitFor(() => expect(harness.api.setEnabled).toHaveBeenCalledWith({ id: 'new-agent', enabled: false }));
    expect(control.element.querySelector('[data-extension-id="new-agent"]')?.textContent).toContain('Off');
    control.destroy();
  });

  it('uses concise health and source labels', () => {
    expect(extensionSettingsLabels.sourceLabel(record('x', { scope: 'project' }))).toBe('Project');
    expect(extensionSettingsLabels.displayState(record('x', { health: { state: 'runtime-error', error: 'crashed' } })))
      .toEqual({ label: 'Needs attention', tone: 'warning', detail: 'crashed' });
  });
});
