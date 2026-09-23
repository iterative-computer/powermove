import type { ExtensionRecord, ExtensionsBridge, ExtensionsChangedEvent } from '../../../../shared/extensions';
import { createSettingsSection } from './settings-section';

export interface ExtensionSettingsControl {
  element: HTMLElement;
  destroy(): void;
}

type DisplayState = { label: string; tone: 'ok' | 'quiet' | 'warning' | 'setup'; detail?: string };

function displayState(record: ExtensionRecord): DisplayState {
  const health = record.health;
  if (!record.enabled || health.state === 'disabled') return { label: 'Off', tone: 'quiet' };
  if (health.state === 'ok') return { label: 'Active', tone: 'ok' };
  if (health.state === 'replaced') return { label: 'Replaced', tone: 'quiet', detail: `Replaced by ${health.by}` };
  if (health.state === 'needs-update') return { label: 'Update needed', tone: 'warning', detail: health.error };
  if (health.state === 'needs-setup') return { label: 'Needs setup', tone: 'setup' };
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

const CAPABILITIES: Record<string, string> = {
  inspector: 'Properties controls', media: 'Media import',
  panels: 'Panels', commands: 'Commands', keybindings: 'Keyboard shortcuts', effects: 'Effects', transitions: 'Transitions',
  layers: 'Layer types', themes: 'Appearance', palette: 'Command search', menus: 'Menus', status: 'Status bar', hooks: 'Editor behavior'
};

const el = <K extends keyof HTMLElementTagNameMap>(tag: K, className = '', text = ''): HTMLElementTagNameMap[K] => {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text) node.textContent = text;
  return node;
};

/** The values sheet (vars/setup.ts), installed on PM at boot. */
function openSetup(record: ExtensionRecord): void {
  (window as any).PM?.Vars?.openSetup?.(record);
}

function hasVars(record: ExtensionRecord): boolean {
  return record.scope === 'user' && (record.manifest?.vars?.length ?? 0) > 0;
}

function icon(name: string): Element | null {
  const svg = (window as any).PM?.icon?.(name);
  return svg instanceof Element ? svg : null;
}

/**
 * The Extensions page: a list where every row is a destination, and a detail
 * view for one extension with its state, what it contributes, its files and
 * the delete action. Both views paint from the same records.
 */
export interface ExtensionSettingsOptions {
  /** Settings hides the built-ins; the Store's Installed page lists them too. */
  includeBuiltin?: boolean;
}

export function createExtensionSettingsControl(
  api: ExtensionsBridge | undefined = window.powermove?.extensions,
  options: ExtensionSettingsOptions = {}
): ExtensionSettingsControl {
  const element = el('div', 'settings-extension-root');
  const section = createSettingsSection('Installed', 'Loading…');
  const summary = section.summary;
  const list = section.body;
  list.setAttribute('role', 'list');
  list.setAttribute('aria-label', 'Extensions');
  const detail = el('div', 'settings-extension-detail');
  detail.hidden = true;
  element.append(section.element, detail);

  let alive = true;
  let stop = (): void => undefined;
  let openId: string | null = null;
  let pendingDelete: string | null = null;
  const busy = new Set<string>();

  const toggleFor = (record: ExtensionRecord, onError: (message: string) => void): HTMLButtonElement => {
    const toggle = el('button', record.enabled ? 'toggle on' : 'toggle');
    toggle.type = 'button';
    toggle.disabled = busy.has(record.id);
    toggle.setAttribute('aria-pressed', String(record.enabled));
    toggle.setAttribute('aria-label', `${record.enabled ? 'Disable' : 'Enable'} ${nameOf(record)}`);
    const knob = el('i');
    knob.setAttribute('aria-hidden', 'true');
    toggle.append(knob);
    toggle.addEventListener('click', async (event) => {
      event.stopPropagation();
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
        onError(error instanceof Error ? error.message : 'The extension could not be updated.');
      }
    });
    return toggle;
  };

  const setupButton = (record: ExtensionRecord, label: string, primary = true): HTMLButtonElement => {
    const button = el('button', primary ? 'btn pri settings-extension-setup' : 'btn settings-extension-setup', label);
    button.type = 'button';
    button.setAttribute('aria-label', label === 'Set up' ? `Set up ${nameOf(record)}` : `Variables for ${nameOf(record)}`);
    button.addEventListener('click', (event) => {
      event.stopPropagation();
      openSetup(record);
    });
    return button;
  };

  const dotFor = (tone: DisplayState['tone']): HTMLElement => {
    const dot = el('i', 'settings-extension-dot');
    dot.dataset.tone = tone;
    dot.setAttribute('aria-hidden', 'true');
    return dot;
  };

  const renderList = (records: ExtensionRecord[], sorted: ExtensionRecord[]): void => {
    summary.textContent = `${sorted.length} ${sorted.length === 1 ? 'extension' : 'extensions'}`;
    list.replaceChildren();
    if (sorted.length === 0) {
      list.append(el('div', 'settings-extension-empty', 'No extensions installed.'));
      return;
    }
    const resolveName = (id: string): string => nameOf(records.find(item => item.id === id) ?? { id } as ExtensionRecord);
    for (const record of sorted) {
      const state = displayState(record);
      if (record.health.state === 'replaced') state.detail = `Replaced by ${resolveName(record.health.by)}`;
      const row = el('div', 'settings-extension-row');
      row.setAttribute('role', 'listitem');
      row.dataset.extensionId = record.id;
      row.title = `${record.id}${record.manifest?.version ? ` · v${record.manifest.version}` : ''}`;

      const open = el('button', 'settings-extension-open');
      open.type = 'button';
      open.setAttribute('aria-label', `Open ${nameOf(record)}`);
      const copy = el('div', 'settings-extension-copy');
      const heading = el('div', 'settings-extension-name');
      heading.append(dotFor(state.tone), el('b', '', nameOf(record)));
      if (record.scope !== 'builtin' && record.manifest?.author !== 'agent') heading.append(el('span', 'settings-extension-tag', sourceLabel(record)));
      copy.append(heading);
      const hint = state.tone === 'setup' ? state.label : state.detail ?? record.manifest?.description ?? '';
      if (hint) {
        const line = el('span', state.tone === 'warning' ? 'settings-extension-error' : state.tone === 'setup' ? 'settings-extension-setup-note' : '', hint);
        line.title = hint;
        copy.append(line);
      }
      open.append(copy);

      const controls = el('div', 'settings-extension-controls');
      if (state.tone === 'warning') controls.append(el('span', `settings-extension-status is-${state.tone}`, state.label));
      if (state.tone === 'setup') controls.append(setupButton(record, 'Set up'));
      controls.append(toggleFor(record, (message) => { summary.textContent = message; }));
      const chevron = el('i', 'settings-extension-chevron');
      chevron.setAttribute('aria-hidden', 'true');
      const glyph = icon('chev');
      if (glyph) chevron.append(glyph);
      controls.append(chevron);

      const show = (): void => { openId = record.id; pendingDelete = null; render(records); };
      open.addEventListener('click', show);
      row.addEventListener('click', (event) => {
        if ((event.target as HTMLElement).closest('.toggle, .settings-extension-open, .settings-extension-setup')) return;
        show();
      });
      row.append(open, controls);
      list.append(row);
    }
  };

  const renderDetail = (record: ExtensionRecord, records: ExtensionRecord[]): void => {
    const resolveName = (id: string): string => nameOf(records.find(item => item.id === id) ?? { id } as ExtensionRecord);
    const state = displayState(record);
    if (record.health.state === 'replaced') state.detail = `Replaced by ${resolveName(record.health.by)}`;
    detail.replaceChildren();
    detail.dataset.extensionId = record.id;

    const back = el('button', 'settings-extension-back');
    back.type = 'button';
    const backGlyph = icon('chev');
    if (backGlyph) back.append(backGlyph);
    back.append(document.createTextNode('Extensions'));
    back.addEventListener('click', () => { openId = null; pendingDelete = null; render(records); });

    const head = el('header', 'settings-extension-head');
    const title = el('div', 'settings-extension-title');
    title.append(dotFor(state.tone), el('h3', '', nameOf(record)));
    if (record.scope !== 'builtin' && record.manifest?.author !== 'agent') title.append(el('span', 'settings-extension-tag', sourceLabel(record)));
    head.append(title);
    if (record.manifest?.description) head.append(el('p', '', record.manifest.description));

    const rowOf = (label: string, value: string | HTMLElement, tone: '' | 'warning' | 'replacement' = ''): HTMLElement => {
      const row = el('div', 'settings-row');
      const copy = el('div', 'settings-copy');
      copy.append(el('b', '', label));
      if (typeof value === 'string') {
        copy.append(el('span', tone === 'warning' ? 'settings-extension-error' : tone === 'replacement' ? 'is-replacement' : '', value));
        row.append(copy);
      } else {
        row.append(copy, value);
      }
      return row;
    };

    const status = createSettingsSection('Status');
    const stateRow = el('div', 'settings-row');
    const stateCopy = el('div', 'settings-copy');
    stateCopy.append(el('b', '', 'Enabled'));
    const stateLine = el('span', state.tone === 'warning' ? 'settings-extension-error' : '', state.detail ?? state.label);
    stateCopy.append(stateLine);
    const stateControls = el('div', 'settings-extension-controls');
    if (state.tone === 'warning') stateControls.append(el('span', `settings-extension-status is-${state.tone}`, state.label));
    stateControls.append(toggleFor(record, (message) => { stateLine.textContent = message; }));
    stateRow.append(stateCopy, stateControls);
    status.body.append(stateRow);

    const about = createSettingsSection('About');
    const version = record.manifest?.version ? ` · v${record.manifest.version}` : '';
    about.body.append(rowOf('Identifier', `${record.id}${version}`));
    const replaces = record.manifest?.replaces ?? [];
    if (replaces.length) about.body.append(rowOf(record.enabled && state.label === 'Active' ? 'Replaces' : 'When active, replaces', replaces.map(resolveName).join(', '), 'replacement'));
    const declared = record.manifest?.contributes ?? [];
    if (declared.length) about.body.append(rowOf('Includes', declared.map(kind => CAPABILITIES[kind] ?? kind).join(' · ')));
    const dependencies = record.manifest?.dependsOn ?? [];
    if (dependencies.length) about.body.append(rowOf('Requires', dependencies.map(resolveName).join(', ')));
    const dependents = records.filter(item => item.enabled && item.manifest?.dependsOn?.includes(record.id));
    if (dependents.length) about.body.append(rowOf('Used by', dependents.map(nameOf).join(', ')));
    if (record.manifest?.forkedFrom) about.body.append(rowOf('Based on', record.manifest.forkedFrom));
    const kernel = (window as any).PM?.Kernel;
    for (const [key, label] of [['panels', 'Panels'], ['commands', 'Commands'], ['keybindings', 'Shortcuts'], ['effects', 'Effects'], ['transitions', 'Transitions'], ['layerTypes', 'Layer types'], ['themes', 'Themes']] as const) {
      const entries = kernel?.[key]?.entries?.() ?? [];
      const owned = entries.filter((entry: any) => entry.ownerId === record.id);
      if (!owned.length) continue;
      about.body.append(rowOf(label, owned.map((entry: any) => entry.item.title ?? entry.item.label ?? entry.item.key ?? entry.id).join(', ')));
    }

    detail.append(back, head, status.element);

    if (hasVars(record)) {
      const values = createSettingsSection('Setup');
      const waiting = record.health.state === 'needs-setup';
      const valuesRow = rowOf('Values', setupButton(record, waiting ? 'Set up' : 'Variables…', waiting));
      if (waiting) valuesRow.querySelector('.settings-copy')!.append(el('span', '', 'Turns on once required values are set.'));
      values.body.append(valuesRow);
      detail.append(values.element);
    }

    detail.append(about.element);

    if (record.scope !== 'builtin') {
      const files = createSettingsSection('Files');
      const reveal = el('button', 'btn', 'Show in Finder');
      reveal.type = 'button';
      const filesRow = rowOf('Extension files', reveal);
      const filesHint = el('span', '', record.dir || 'On this computer.');
      filesRow.querySelector('.settings-copy')!.append(filesHint);
      reveal.addEventListener('click', async () => {
        try { await api?.reveal({ id: record.id }); }
        catch (error) { if (alive) filesHint.textContent = error instanceof Error ? error.message : 'Could not show extension files.'; }
      });
      files.body.append(filesRow);
      detail.append(files.element);
    }

    if (record.scope === 'user') {
      const danger = createSettingsSection('Remove');
      const remove = el('button', 'btn settings-extension-delete', 'Delete…');
      remove.type = 'button';
      remove.setAttribute('aria-label', `Delete ${nameOf(record)}`);
      remove.disabled = busy.has(record.id);
      remove.addEventListener('click', () => { pendingDelete = record.id; render(records); });
      const removeRow = rowOf('Delete extension', remove);
      removeRow.querySelector('.settings-copy')!.append(el('span', '', 'Removes its files from this computer.'));
      danger.body.append(removeRow);
      if (pendingDelete === record.id) {
        const confirmation = el('div', 'settings-extension-confirm');
        confirmation.setAttribute('role', 'group');
        confirmation.setAttribute('aria-label', `Delete ${nameOf(record)}?`);
        const explanation = el('p', '',
          `Delete ${nameOf(record)} from this computer? Its extension files will be permanently removed.` +
          (replaces.length ? ` ${replaces.map(resolveName).join(', ')} can take over again if enabled.` : '') +
          (declared.some(kind => ['layers', 'effects', 'transitions'].includes(kind)) ? ' Projects using its layer types or effects may need it reinstalled to render correctly.' : '') +
          (dependents.length ? ` ${dependents.map(nameOf).join(', ')} depend on it and may stop working.` : ''));
        const cancel = el('button', 'btn', 'Cancel');
        cancel.type = 'button';
        cancel.addEventListener('click', () => { pendingDelete = null; render(records); detail.querySelector<HTMLButtonElement>('.settings-extension-delete')?.focus(); });
        const confirm = el('button', 'btn settings-extension-delete is-confirm', 'Delete extension');
        confirm.type = 'button';
        confirm.disabled = cancel.disabled = busy.has(record.id);
        confirm.addEventListener('click', async () => {
          if (!api || busy.has(record.id)) return;
          busy.add(record.id); confirm.disabled = cancel.disabled = true;
          try {
            const updated = await api.remove({ id: record.id });
            busy.delete(record.id); pendingDelete = null; openId = null; render(updated);
          } catch (error) {
            busy.delete(record.id); render(records);
            const line = detail.querySelector<HTMLElement>('.settings-extension-confirm p');
            if (line) line.textContent = error instanceof Error ? error.message : 'The extension could not be deleted.';
          }
        });
        const actions = el('div', 'settings-extension-confirm-actions');
        actions.append(cancel, confirm);
        confirmation.append(explanation, actions);
        danger.body.append(confirmation);
        queueMicrotask(() => { if (alive && confirmation.isConnected) cancel.focus(); });
      }
      detail.append(danger.element);
    }
  };

  const render = (records: ExtensionRecord[]): void => {
    if (!alive) return;
    const sorted = records.filter(record => options.includeBuiltin || record.scope !== 'builtin').sort(compareRecords);
    const open = openId ? sorted.find(record => record.id === openId) ?? null : null;
    if (openId && !open) openId = null;
    renderList(records, sorted);
    section.element.hidden = !!open;
    detail.hidden = !open;
    if (open) renderDetail(open, records);
    else { detail.replaceChildren(); delete detail.dataset.extensionId; }
  };

  const refresh = async (): Promise<void> => {
    if (!api) {
      summary.textContent = 'Restart Powermove to manage extensions.';
      list.replaceChildren(el('div', 'settings-extension-empty', 'Extensions are unavailable in this session.'));
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
    element,
    destroy: () => {
      alive = false;
      stop();
    }
  };
}

export const extensionSettingsLabels = { displayState, sourceLabel };
