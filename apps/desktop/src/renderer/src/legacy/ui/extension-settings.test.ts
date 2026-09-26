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
  it('shows added extensions and hides built-ins and group headings', async () => {
    const harness = bridge([
      record('builtin-ext', { scope: 'builtin', manifest: { id: 'builtin-ext', name: 'Builtin', version: '1.0.0', apiVersion: 1, author: 'powermove' } }),
      record('agent-ext', { manifest: { id: 'agent-ext', name: 'Agent tool', version: '2.0.0', apiVersion: 1, author: 'agent' } }),
      record('disabled-ext', { enabled: false, health: { state: 'disabled' } }),
      record('old-timeline', { health: { state: 'replaced', by: 'new-timeline' } }),
      record('broken-ext', { manifest: null, bundleUrl: null, bundleHash: null, health: { state: 'manifest-error', error: 'Invalid manifest' } })
    ]);
    const control = createExtensionSettingsControl(harness.api);

    await vi.waitFor(() => expect(control.element.querySelectorAll('[data-extension-id]')).toHaveLength(4));
    expect(control.element.textContent).toContain('4 extensions');
    expect(control.element.querySelector('[data-extension-id="agent-ext"] .settings-extension-tag')).toBeNull();
    expect(control.element.querySelector('.settings-extension-group')).toBeNull();
    expect(control.element.querySelector('[data-extension-id="builtin-ext"]')).toBeNull();
    expect(control.element.querySelector('[data-extension-id="disabled-ext"] .toggle')?.getAttribute('aria-pressed')).toBe('false');
    expect(control.element.querySelector('[data-extension-id="broken-ext"] .settings-extension-status')?.textContent).toBe('Needs attention');
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

    const toggle = control.element.querySelector<HTMLButtonElement>('[data-extension-id="new-agent"] .toggle');
    if (!toggle) throw new Error('New agent extension toggle was not rendered.');
    expect(toggle.classList.contains('on')).toBe(true);
    toggle.click();
    await vi.waitFor(() => expect(harness.api.setEnabled).toHaveBeenCalledWith({ id: 'new-agent', enabled: false }));
    await vi.waitFor(() => expect(control.element
      .querySelector('[data-extension-id="new-agent"] .toggle')?.getAttribute('aria-pressed')).toBe('false'));
    control.destroy();
  });

  it('explains replacements and dependencies and requires confirmation before deletion', async () => {
    const base = record('viewer', { scope: 'builtin', manifest: { id: 'viewer', name: 'Composition viewer', version: '1.0.0', apiVersion: 1 } });
    const custom = record('custom-viewer', { manifest: { id: 'custom-viewer', name: 'Custom viewer', version: '1.0.0', apiVersion: 1, replaces: ['viewer'], contributes: ['panels', 'keybindings'] } });
    const harness = bridge([base, custom]);
    harness.api.remove.mockResolvedValue([base]);
    const control = createExtensionSettingsControl(harness.api);
    await vi.waitFor(() => expect(control.element.querySelector('[data-extension-id="custom-viewer"] .settings-extension-open')).not.toBeNull());
    expect(control.element.querySelector('[data-extension-id="viewer"]')).toBeNull();
    control.element.querySelector<HTMLButtonElement>('[data-extension-id="custom-viewer"] .settings-extension-open')!.click();
    expect(control.element.textContent).toContain('Replaces');
    expect(control.element.textContent).toContain('Composition viewer');
    expect(control.element.textContent).toContain('Panels · Keyboard shortcuts');
    const remove = () => control.element.querySelector<HTMLButtonElement>('[aria-label="Delete Custom viewer"]')!;
    remove().click();
    expect(harness.api.remove).not.toHaveBeenCalled();
    expect(control.element.textContent).toContain('permanently removed');
    const buttons = () => Array.from(control.element.querySelectorAll<HTMLButtonElement>('.settings-extension-confirm button'));
    buttons().find(button => button.textContent === 'Cancel')!.click();
    expect(control.element.querySelector('.settings-extension-confirm')).toBeNull();
    remove().click();
    buttons().find(button => button.textContent === 'Delete extension')!.click();
    await vi.waitFor(() => expect(harness.api.remove).toHaveBeenCalledExactlyOnceWith({ id: 'custom-viewer' }));
    await vi.waitFor(() => expect(control.element.querySelector('[data-extension-id="custom-viewer"]')).toBeNull());
    control.destroy();
  });

  it('keeps the extension and reports a failed delete', async () => {
    const harness = bridge([record('failed')]);
    harness.api.remove.mockRejectedValue(new Error('Permission denied'));
    const control = createExtensionSettingsControl(harness.api);
    await vi.waitFor(() => expect(control.element.querySelector('[data-extension-id="failed"] .settings-extension-open')).not.toBeNull());
    control.element.querySelector<HTMLButtonElement>('[data-extension-id="failed"] .settings-extension-open')!.click();
    expect(control.element.querySelector('[aria-label="Delete failed"]')).not.toBeNull();
    control.element.querySelector<HTMLButtonElement>('[aria-label="Delete failed"]')!.click();
    control.element.querySelector<HTMLButtonElement>('.settings-extension-confirm .settings-extension-delete')!.click();
    await vi.waitFor(() => expect(control.element.textContent).toContain('Permission denied'));
    expect(control.element.querySelector('[data-extension-id="failed"]')).not.toBeNull();
    expect(control.element.querySelector<HTMLButtonElement>('.settings-extension-confirm .settings-extension-delete')!.disabled).toBe(false);
    control.destroy();
  });

  it('uses concise health and source labels', () => {
    expect(extensionSettingsLabels.sourceLabel(record('x', { scope: 'project' }))).toBe('Project');
    expect(extensionSettingsLabels.displayState(record('x', { health: { state: 'runtime-error', error: 'crashed' } })))
      .toEqual({ label: 'Needs attention', tone: 'warning', detail: 'crashed' });
  });
});

describe('extension values in settings', () => {
  const vars = [{ key: 'API_KEY', label: 'API key', secret: true, required: true }];

  it('offers Set up on a row that needs setup and Variables… on the page of a working one', async () => {
    const openSetup = vi.fn();
    (window as any).PM = { Vars: { openSetup } };
    const waiting = record('colour-match', {
      manifest: { id: 'colour-match', name: 'Colour match', version: '1.0.0', apiVersion: 2, author: 'user', vars },
      bundleUrl: null,
      bundleHash: null,
      health: { state: 'needs-setup', missing: ['API_KEY'] }
    });
    const working = record('weather', {
      manifest: { id: 'weather', name: 'Weather', version: '1.0.0', apiVersion: 2, author: 'user', vars }
    });
    const harness = bridge([waiting, working]);
    const control = createExtensionSettingsControl(harness.api);

    await vi.waitFor(() => expect(control.element.querySelectorAll('[data-extension-id]')).toHaveLength(2));
    const row = control.element.querySelector('[data-extension-id="colour-match"]')!;
    expect(row.textContent).toContain('Needs setup');
    const setUp = row.querySelector<HTMLButtonElement>('.settings-extension-setup')!;
    expect(setUp.textContent).toBe('Set up');
    expect(setUp.classList.contains('pri')).toBe(true);
    setUp.click();
    expect(openSetup).toHaveBeenCalledWith(expect.objectContaining({ id: 'colour-match' }));
    expect(control.element.querySelector('.settings-extension-detail')!.hasAttribute('hidden')).toBe(true);
    expect(control.element.querySelector('[data-extension-id="weather"] .settings-extension-setup')).toBeNull();

    control.element.querySelector<HTMLButtonElement>('[data-extension-id="weather"] .settings-extension-open')!.click();
    const variables = control.element.querySelector<HTMLButtonElement>('.settings-extension-detail .settings-extension-setup')!;
    expect(variables.textContent).toBe('Variables…');
    variables.click();
    expect(openSetup).toHaveBeenLastCalledWith(expect.objectContaining({ id: 'weather' }));
    expect(extensionSettingsLabels.displayState(waiting)).toEqual({ label: 'Needs setup', tone: 'setup' });
    control.destroy();
    delete (window as any).PM;
  });
});
