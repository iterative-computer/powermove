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
  if (record.manifest?.author === 'agent') return 'AI-created';
  if (record.scope === 'project') return 'Project';
  return 'Custom';
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
  const section = createSettingsSection('Installed', 'Loading…');
  section.element.classList.add('is-fill');
  const summary = section.summary;
  const list = section.body;
  list.setAttribute('role', 'list');
  list.setAttribute('aria-label', 'Extensions');

  let alive = true;
  let stop = (): void => undefined;
  let pendingDelete: string | null = null;
  const busy = new Set<string>();
  const expanded = new Set<string>();
  const intro = document.createElement('p');
  intro.className = 'settings-note';
  intro.textContent = 'Manage extensions and turn them on or off.';
  section.element.insertBefore(intro, list);


  const render = (records: ExtensionRecord[]): void => {
    if (!alive) return;
    const sorted = records.filter(record => record.scope !== 'builtin').sort(compareRecords);
    summary.textContent = `${sorted.length} ${sorted.length === 1 ? 'extension' : 'extensions'}`;
    list.replaceChildren();
    if (sorted.length === 0) {
      const empty = document.createElement('div');
      empty.className = 'settings-extension-empty';
      empty.textContent = 'No extensions installed.';
      list.append(empty);
      return;
    }

    const resolveName = (id: string): string => nameOf(records.find(item => item.id === id) ?? { id } as ExtensionRecord);
    for (const record of sorted) {
      const state = displayState(record);
      if (record.health.state === 'replaced') state.detail = `Replaced by ${resolveName(record.health.by)}`;
      const row = document.createElement('div');
      row.className = 'settings-extension-row';
      row.setAttribute('role', 'listitem');
      row.dataset.extensionId = record.id;
      const copy = document.createElement('div');
      copy.className = 'settings-extension-copy';
      const heading = document.createElement('div');
      heading.className = 'settings-extension-name';
      const name = document.createElement('b');
      name.textContent = nameOf(record);
      heading.append(name);
      if (record.scope !== 'builtin' && record.manifest?.author !== 'agent') {
        const tag = document.createElement('span');
        tag.className = 'settings-extension-tag';
        tag.textContent = sourceLabel(record);
        heading.append(tag);
      }
      const version = record.manifest?.version ? ` · v${record.manifest.version}` : '';
      row.title = `${record.id}${version}`;
      copy.append(heading);
      if (state.detail || record.manifest?.description) {
        const detail = document.createElement('span');
        detail.className = state.tone === 'warning' ? 'settings-extension-error' : '';
        detail.textContent = state.detail ?? record.manifest?.description ?? '';
        detail.title = detail.textContent;
        copy.append(detail);
      }

      const technicalInfo = document.createElement('div');
      const addLine = (label: string, text: string, className = ''): void => {
        const line = document.createElement('div');
        line.className = `settings-extension-impact ${className}`;
        const title = document.createElement('strong');
        title.textContent = `${label} `;
        line.append(title, document.createTextNode(text));
        technicalInfo.append(line);
      };
      const replaces = record.manifest?.replaces ?? [];
      if (replaces.length) addLine(record.enabled && state.label === 'Active' ? 'Replaces' : 'When active, replaces', replaces.map(resolveName).join(', '), 'is-replacement');
      const capabilities: Record<string, string> = { panels: 'Panels', commands: 'Commands', keybindings: 'Keyboard shortcuts', effects: 'Effects', transitions: 'Transitions', layers: 'Layer types', themes: 'Appearance', palette: 'Command search', menus: 'Menus', status: 'Status bar', hooks: 'Editor behavior' };
      const declared = record.manifest?.contributes ?? [];
      if (declared.length) addLine('Includes', declared.map(kind => capabilities[kind] ?? kind).join(' · '));
      const dependencies = record.manifest?.dependsOn ?? [];
      if (dependencies.length) addLine('Requires', dependencies.map(resolveName).join(', '));
      const dependents = records.filter(item => item.enabled && item.manifest?.dependsOn?.includes(record.id));
      if (dependents.length) addLine('Used by', dependents.map(nameOf).join(', '));

      const details = document.createElement('details');
      details.className = 'settings-extension-details';
      details.open = expanded.has(record.id);
      details.addEventListener('toggle', () => { if (details.open) expanded.add(record.id); else expanded.delete(record.id); });
      const detailLabel = document.createElement('summary');
      detailLabel.textContent = 'Details';
      details.append(detailLabel);
      const info = document.createElement('div');
      info.textContent = `${record.id}${version}`;
      details.append(info, technicalInfo);
      const kernel = (window as any).PM?.Kernel;
      for (const [key, label] of [['panels', 'Panels'], ['commands', 'Commands'], ['keybindings', 'Shortcuts'], ['effects', 'Effects'], ['transitions', 'Transitions'], ['layerTypes', 'Layer types'], ['themes', 'Themes']] as const) {
        const entries = kernel?.[key]?.entries?.() ?? [];
        const owned = entries.filter((entry: any) => entry.ownerId === record.id);
        if (!owned.length) continue;
        const contribution = document.createElement('div');
        contribution.textContent = `${label}: ${owned.map((entry: any) => entry.item.title ?? entry.item.label ?? entry.item.key ?? entry.id).join(', ')}`;
        details.append(contribution);
      }
      if (record.manifest?.forkedFrom) {
        const origin = document.createElement('div');
        origin.textContent = `Based on ${record.manifest.forkedFrom}`;
        details.append(origin);
      }
      if (record.scope !== 'builtin') {
        const reveal = document.createElement('button');
        reveal.type = 'button'; reveal.className = 'btn'; reveal.textContent = 'Show in Finder';
        reveal.onclick = async () => {
          try { await api?.reveal({ id: record.id }); }
          catch (error) { if (alive) summary.textContent = error instanceof Error ? error.message : 'Could not show extension files.'; }
        };
        details.append(reveal);
      }
      copy.append(details);
      const controls = document.createElement('div');
      controls.className = 'settings-extension-controls';
      const toggle = document.createElement('button');
      toggle.type = 'button';
      toggle.disabled = busy.has(record.id);
      toggle.className = record.enabled ? 'toggle on' : 'toggle';
      toggle.setAttribute('aria-pressed', String(record.enabled));
      toggle.setAttribute('aria-label', `${record.enabled ? 'Disable' : 'Enable'} ${nameOf(record)}`);
      const knob = document.createElement('i');
      knob.setAttribute('aria-hidden', 'true');
      toggle.append(knob);
      toggle.addEventListener('click', async () => {
        if (!api || toggle.disabled) return;
        const next = !record.enabled;
        toggle.disabled = true;
        busy.add(record.id);
        toggle.classList.toggle('on', next);
        toggle.setAttribute('aria-pressed', String(next));
        try {
          const updated = await api.setEnabled({ id: record.id, enabled: next });
          busy.delete(record.id);
          render(updated);
        } catch (error) {
          busy.delete(record.id);
          toggle.classList.toggle('on', record.enabled);
          toggle.setAttribute('aria-pressed', String(record.enabled));
          toggle.disabled = false;
          summary.textContent = error instanceof Error ? error.message : 'The extension could not be updated.';
        }
      });
      // Distinguish enabled intent from active, replaced, and failed extensions.
      {
        const status = document.createElement('span');
        status.className = `settings-extension-status is-${state.tone}`;
        status.textContent = state.label;
        controls.append(status);
      }
      controls.append(toggle);
      if (record.scope === 'user') {
        const remove = document.createElement('button');
        remove.type = 'button';
        remove.className = 'settings-extension-delete';
        remove.textContent = 'Delete…';
        remove.setAttribute('aria-label', `Delete ${nameOf(record)}`);
        remove.disabled = busy.has(record.id);
        remove.onclick = () => { pendingDelete = record.id; render(records); };
        controls.append(remove);
      }
      if (pendingDelete === record.id) {
        const confirmation = document.createElement('div');
        confirmation.className = 'settings-extension-confirm';
        confirmation.setAttribute('role', 'group');
        confirmation.setAttribute('aria-label', `Delete ${nameOf(record)}?`);
        const explanation = document.createElement('p');
        explanation.textContent = `Delete ${nameOf(record)} from this computer? Its extension files will be permanently removed.` +
          (replaces.length ? ` ${replaces.map(resolveName).join(', ')} can take over again if enabled.` : '') +
          (declared.some(kind => ['layers', 'effects', 'transitions'].includes(kind)) ? ' Projects using its layer types or effects may need it reinstalled to render correctly.' : '') +
          (dependents.length ? ` ${dependents.map(nameOf).join(', ')} depend on it and may stop working.` : '');
        const cancel = document.createElement('button');
        cancel.type = 'button'; cancel.className = 'btn'; cancel.textContent = 'Cancel';
        cancel.onclick = () => { pendingDelete = null; render(records); list.querySelector<HTMLButtonElement>(`[data-extension-id="${record.id}"] .settings-extension-delete`)?.focus(); };
        const confirm = document.createElement('button');
        confirm.type = 'button'; confirm.className = 'btn settings-extension-delete'; confirm.textContent = 'Delete extension';
        confirm.disabled = cancel.disabled = busy.has(record.id);
        confirm.onclick = async () => {
          if (!api || busy.has(record.id)) return;
          busy.add(record.id); confirm.disabled = cancel.disabled = true;
          try {
            const updated = await api.remove({ id: record.id });
            busy.delete(record.id); pendingDelete = null; render(updated);
          } catch (error) {
            busy.delete(record.id); render(records);
            summary.textContent = error instanceof Error ? error.message : 'The extension could not be deleted.';
          }
        };
        confirmation.append(explanation, cancel, confirm);
        copy.append(confirmation);
        queueMicrotask(() => { if (alive && confirmation.isConnected) cancel.focus(); });
      }
      row.append(copy, controls);
      list.append(row);
    }
  };

  const refresh = async (): Promise<void> => {
    if (!api) {
      summary.textContent = 'Restart Powermove to manage extensions.';
      const empty = document.createElement('div');
      empty.className = 'settings-extension-empty';
      empty.textContent = 'Extensions are unavailable in this session.';
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
