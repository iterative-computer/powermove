import type { ExtensionRecord, ExtensionsBridge, ExtensionsChangedEvent } from '../../../../shared/extensions';
import { createSettingsSection } from './settings-tabs';

export interface ExtensionSettingsControl {
  element: HTMLElement;
  destroy(): void;
}

type DisplayState = { label: string; tone: 'ok' | 'quiet' | 'warning'; detail?: string };

function displayState(record: ExtensionRecord): DisplayState {
  const health = record.health;
  if (!record.enabled || health.state === 'disabled') return { label: 'Off', tone: 'quiet' };
  if (health.state === 'ok') return { label: 'Active', tone: 'ok' };
  if (health.state === 'replaced') return { label: 'Replaced', tone: 'quiet', detail: `Replaced by ${health.by}` };
  if (health.state === 'needs-update') return { label: 'Update needed', tone: 'warning', detail: health.error };
  return { label: 'Needs attention', tone: 'warning', detail: health.error };
}

function sourceLabel(record: ExtensionRecord): string {
  if (record.scope === 'builtin') return 'Built in';
  if (record.manifest?.author === 'agent') return 'Agent';
  if (record.scope === 'project') return 'Project';
  return 'User';
}

function nameOf(record: ExtensionRecord): string {
  return record.manifest?.name ?? record.id;
}

function compareRecords(a: ExtensionRecord, b: ExtensionRecord): number {
  const rank = (record: ExtensionRecord): number => record.scope === 'builtin' ? 2 : record.manifest?.author === 'agent' ? 0 : 1;
  return rank(a) - rank(b) || nameOf(a).localeCompare(nameOf(b));
}

export function createExtensionSettingsControl(
  api: ExtensionsBridge | undefined = window.powermove?.extensions
): ExtensionSettingsControl {
  const section = createSettingsSection('Extensions', 'Loading…');
  const summary = section.summary;
  const list = section.body;
  list.classList.add('settings-extension-list');
  list.setAttribute('role', 'list');
  list.setAttribute('aria-label', 'Extensions');

  let alive = true;
  let stop = (): void => undefined;

  const render = (records: ExtensionRecord[]): void => {
    if (!alive) return;
    const sorted = [...records].sort(compareRecords);
    summary.textContent = `${sorted.length} ${sorted.length === 1 ? 'extension' : 'extensions'} discovered`;
    list.replaceChildren();
    if (sorted.length === 0) {
      const empty = document.createElement('div');
      empty.className = 'settings-extension-empty';
      empty.textContent = 'No extensions were discovered.';
      list.append(empty);
      return;
    }

    for (const record of sorted) {
      const state = displayState(record);
      const row = document.createElement('div');
      row.className = 'settings-extension-row';
      row.setAttribute('role', 'listitem');
      row.dataset.extensionId = record.id;
      const copy = document.createElement('div');
      copy.className = 'settings-extension-copy';
      const name = document.createElement('b');
      name.textContent = nameOf(record);
      const metadata = document.createElement('span');
      const version = record.manifest?.version ? ` · v${record.manifest.version}` : '';
      metadata.textContent = `${record.id} · ${sourceLabel(record)}${version}`;
      copy.append(name, metadata);
      if (state.detail || record.manifest?.description) {
        const detail = document.createElement('span');
        detail.className = state.tone === 'warning' ? 'settings-extension-error' : '';
        detail.textContent = state.detail ?? record.manifest?.description ?? '';
        detail.title = detail.textContent;
        copy.append(detail);
      }

      const controls = document.createElement('div');
      controls.className = 'settings-extension-controls';
      const status = document.createElement('span');
      status.className = `settings-extension-status is-${state.tone}`;
      status.textContent = state.label;
      const toggle = document.createElement('input');
      toggle.type = 'checkbox';
      toggle.className = 'settings-extension-toggle';
      toggle.checked = record.enabled;
      toggle.setAttribute('aria-label', `${record.enabled ? 'Disable' : 'Enable'} ${nameOf(record)}`);
      toggle.addEventListener('change', async () => {
        if (!api) return;
        toggle.disabled = true;
        try {
          render(await api.setEnabled({ id: record.id, enabled: toggle.checked }));
        } catch (error) {
          toggle.checked = record.enabled;
          summary.textContent = error instanceof Error ? error.message : 'The extension could not be updated.';
        } finally {
          toggle.disabled = false;
        }
      });
      controls.append(status, toggle);
      row.append(copy, controls);
      list.append(row);
    }
  };

  const refresh = async (): Promise<void> => {
    if (!api) {
      summary.textContent = 'Restart Powermove once to inspect extensions.';
      const empty = document.createElement('div');
      empty.className = 'settings-extension-empty';
      empty.textContent = 'The extension bridge is not available yet.';
      list.replaceChildren(empty);
      return;
    }
    try {
      render(await api.list());
    } catch (error) {
      if (!alive) return;
      summary.textContent = error instanceof Error ? error.message : 'Extensions could not be loaded.';
      list.replaceChildren();
    }
  };

  if (api) {
    stop = api.onChanged((_event: ExtensionsChangedEvent) => void refresh());
  }
  void refresh();

  return {
    element: section.element,
    destroy: () => {
      alive = false;
      stop();
    }
  };
}

export const extensionSettingsLabels = { displayState, sourceLabel };
