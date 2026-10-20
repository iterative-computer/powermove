import type { ExtensionRecord, ExtensionsBridge, ExtensionsChangedEvent } from '../../../../shared/extensions';
import { createSettingsSection } from './settings-section';
import { bridge as hostBridge } from '../../kernel/bridge';

export interface ExtensionSettingsControl {
  element: HTMLElement;
  destroy(): void;
}

let nextControlId = 0;

type DisplayState = { label: string; tone: 'ok' | 'quiet' | 'warning' | 'setup'; detail?: string };

function displayState(record: ExtensionRecord): DisplayState {
  const health = record.health;
  if (!record.enabled || health.state === 'disabled') return { label: 'Off', tone: 'quiet' };
  if (health.state === 'ok') return { label: 'Active', tone: 'ok' };
  if (health.state === 'replaced') return { label: 'Replaced', tone: 'quiet', detail: `Replaced by ${health.by}` };
  if (health.state === 'needs-update') return { label: 'Update needed', tone: 'warning', detail: health.error };
  if (health.state === 'needs-setup') return { label: 'Needs setup', tone: 'setup' };
  if (health.state === 'needs-trust') return { label: 'Needs full access', tone: 'warning', detail: 'Trust it from the Library to turn it on.' };
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

// Display order is separate from primary placement: concrete content takes
// precedence over a companion panel, command or property-control contribution.
const EXTENSION_GROUPS = [
  { id: 'effects', label: 'Effects', kinds: ['effects', 'transitions'] },
  { id: 'panels', label: 'Panels', kinds: ['panels'] },
  { id: 'appearance', label: 'Appearance', kinds: ['themes'] },
  { id: 'layers', label: 'Layer types', kinds: ['layers'] },
  { id: 'media', label: 'Media', kinds: ['media'] },
  { id: 'tools', label: 'Editor tools', kinds: ['commands', 'inspector', 'keybindings', 'palette', 'menus', 'status', 'hooks'] },
  { id: 'other', label: 'Other extensions', kinds: [] }
] as const;
type ExtensionGroupId = (typeof EXTENSION_GROUPS)[number]['id'];
const PRIMARY_GROUP_ORDER: ExtensionGroupId[] = ['effects', 'appearance', 'layers', 'media', 'panels', 'tools', 'other'];
const GROUP_ICONS: Record<ExtensionGroupId, string> = {
  effects: 'wand', panels: 'grid', appearance: 'sun', layers: 'layers', media: 'image', tools: 'sliders', other: 'puzzle'
};
const categoryNames = Object.fromEntries(EXTENSION_GROUPS.map(group => [group.id, group.label])) as Record<ExtensionGroupId, string>;
function categoriesOf(record: ExtensionRecord): ExtensionGroupId[] {
  const kinds = record.manifest?.contributes ?? [];
  const groups = EXTENSION_GROUPS.filter(group => group.kinds.some(kind => kinds.includes(kind))).map(group => group.id);
  return groups.length ? groups : ['other'];
}
function primaryCategory(record: ExtensionRecord): ExtensionGroupId {
  const groups = categoriesOf(record);
  return PRIMARY_GROUP_ORDER.find(group => groups.includes(group))!;
}

type SubcategoryMetadata = { dir: string; version?: string; hash: string | null; effects: string[]; panels: string[]; appearance: string[] };
// Keep previously observed metadata when disabling an extension unregisters it.
// Cache ownership follows the kernel, and new code/version invalidates its entry.
const metadataByKernel = new WeakMap<object, Map<string, SubcategoryMetadata>>();

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
 * centered dialog for one extension with its state, what it contributes, its files and
 * the delete action. Both views paint from the same records.
 */
export interface ExtensionSettingsOptions {
  /** Settings hides the built-ins; the Store's Installed page lists them too. */
  includeBuiltin?: boolean;
}

export function createExtensionSettingsControl(
  api: ExtensionsBridge | undefined = hostBridge()?.extensions,
  options: ExtensionSettingsOptions = {}
): ExtensionSettingsControl {
  const controlId = `extension-settings-${++nextControlId}`;
  const element = el('div', 'settings-extension-root');
  const section = createSettingsSection('Installed', 'Loading…');
  const summary = section.summary;
  section.element.querySelector('.settings-section-heading')?.remove();
  const list = section.body;
  list.classList.add('settings-extension-list');
  list.tabIndex = 0;
  list.setAttribute('role', 'list');
  list.setAttribute('aria-label', 'Extensions');
  const detail = el('dialog', 'settings-extension-detail');
  detail.setAttribute('aria-label', 'Extension details');
  const toolbar = el('div', 'settings-extension-toolbar');
  const search = el('input', 'settings-input settings-extension-search');
  search.type = 'search'; search.placeholder = 'Search extensions';
  search.setAttribute('aria-label', 'Search extensions');
  const filters = el('div', 'settings-extension-filters');
  filters.setAttribute('role', 'group'); filters.setAttribute('aria-label', 'Filter extensions');
  const types = el('select', 'settings-select');
  types.setAttribute('aria-label', 'Extension type');
  const allTypes = el('option', '', 'All types'); allTypes.value = 'all'; types.append(allTypes);
  const searchRow = el('div', 'settings-extension-search-row');
  searchRow.append(search, types); toolbar.append(searchRow, filters);
  const message = el('div', 'settings-extension-message');
  message.setAttribute('role', 'status'); message.setAttribute('aria-live', 'polite');
  const footer = el('div', 'settings-extension-footer');
  footer.setAttribute('role', 'status');
  section.element.insertBefore(toolbar, list);
  section.element.append(footer, message);
  detail.hidden = true;
  element.append(section.element, detail);

  let alive = true;
  let stop = (): void => undefined;
  let openId: string | null = null;
  let pendingDelete: string | null = null;
  const busy = new Set<string>();
  let current: ExtensionRecord[] = [];
  let filter = 'all';
  let globalQuery = '';
  const collapsed = new Set<string>(EXTENSION_GROUPS.map(group => group.id));
  let returnId: string | null = null;
  let refreshVersion = 0;
  const expandedSubcategories = new Set<string>();
  const kernel = (window as any).PM?.Kernel;
  const metadataCache: Map<string, SubcategoryMetadata> = kernel
    ? metadataByKernel.get(kernel) ?? new Map() : new Map();
  if (kernel) metadataByKernel.set(kernel, metadataCache);
  let effectOrder = new Map<string, number>();
  const unique = (values: unknown[]): string[] => [...new Set(values.filter((value): value is string => typeof value === 'string' && !!value.trim()).map(value => value.trim()))];
  const refreshMetadata = (records: ExtensionRecord[]): void => {
    const read = (registry: any, value: (item: any) => unknown): Map<string, string[]> => {
      const owners = new Map<string, string[]>();
      for (const entry of registry?.entries?.() ?? []) {
        const values = owners.get(entry.ownerId) ?? [];
        values.push(value(entry.item) as string); owners.set(entry.ownerId, values);
      }
      return new Map([...owners].map(([owner, values]) => [owner, unique(values)]));
    };
    // Identical category metadata to FxBrowserPanel, with owner IDs retained.
    const effects = read(kernel?.effects, item => item.group ?? 'Other');
    effectOrder = new Map(unique((kernel?.effects?.entries?.() ?? []).map((entry: any) => entry.item.group ?? 'Other')).map((group, index) => [group, index]));
    const panels = read(kernel?.commands, item => typeof item.category === 'string' && !/^panels?$/i.test(item.category.trim()) ? item.category : undefined);
    const schemes: Record<string, string> = { light: 'Light themes', dark: 'Dark themes', auto: 'Adaptive themes' };
    const appearance = read(kernel?.themes, item => schemes[item.scheme]);
    const ids = new Set(records.map(record => record.id));
    for (const id of metadataCache.keys()) if (!ids.has(id)) metadataCache.delete(id);
    for (const record of records) {
      const previous = metadataCache.get(record.id);
      const valid = previous?.dir === record.dir && previous.version === record.manifest?.version
        && (!record.bundleHash || previous.hash === record.bundleHash);
      const metadata: SubcategoryMetadata = valid ? previous! : {
        dir: record.dir, version: record.manifest?.version, hash: record.bundleHash,
        effects: [], panels: [], appearance: []
      };
      if (effects.has(record.id)) metadata.effects = effects.get(record.id)!;
      if (panels.has(record.id)) metadata.panels = panels.get(record.id)!;
      if (appearance.has(record.id)) metadata.appearance = appearance.get(record.id)!;
      metadataCache.set(record.id, metadata);
    }
  };
  const subcategoryTerms = (record: ExtensionRecord, kind: string): string[] => {
    const metadata = metadataCache.get(record.id);
    if (kind === 'effects') return unique([...(metadata?.effects ?? []),
      ...(record.manifest?.contributes?.includes('transitions') ? ['Transitions'] : [])]);
    if (kind === 'panels') return metadata?.panels ?? [];
    if (kind === 'appearance') return metadata?.appearance ?? [];
    return [];
  };
  const subcategoryOf = (record: ExtensionRecord, kind: string): string => {
    const terms = subcategoryTerms(record, kind);
    if (terms.length > 1) return kind === 'appearance' ? 'Multiple appearances' : 'Multiple categories';
    return terms[0] ?? (kind === 'panels' ? 'Other panels' : kind === 'appearance' ? 'Other appearances' : 'Other');
  };
  let typeOptionsKey = '';
  const updateTypeOptions = (records: ExtensionRecord[]): void => {
    const available = new Set(records.flatMap(categoriesOf));
    const groups = EXTENSION_GROUPS.filter(group => available.has(group.id));
    const key = groups.map(group => group.id).join(',');
    const value = types.value;
    if (key !== typeOptionsKey) {
      typeOptionsKey = key;
      types.replaceChildren(allTypes, ...groups.map(group => {
        const option = el('option', '', group.label); option.value = group.id; return option;
      }));
    }
    types.value = available.has(value as ExtensionGroupId) ? value : 'all';
  };
  // A type filter includes secondary capabilities and shows each match once in
  // that selected category. All types restores stable, single primary placement.
  const category = (record: ExtensionRecord): ExtensionGroupId => types.value !== 'all'
    ? types.value as ExtensionGroupId : primaryCategory(record);
  const searchable = (record: ExtensionRecord): string => [nameOf(record), record.id, record.manifest?.description,
    sourceLabel(record), displayState(record).label, ...categoriesOf(record).map(group => categoryNames[group]), ...(record.manifest?.contributes ?? []).map(kind => CAPABILITIES[kind] ?? kind),
    ...['effects', 'panels', 'appearance'].flatMap(kind => subcategoryTerms(record, kind))].join(' ').toLowerCase();
  const restoreRowFocus = (): void => {
    const row = Array.from(list.querySelectorAll<HTMLElement>('[data-extension-id]')).find(row => row.dataset.extensionId === returnId);
    (row?.querySelector<HTMLButtonElement>('.settings-extension-open') ?? search).focus({ preventScroll: true });
  };
  const closeDetail = (): void => {
    if (detail.open) detail.close();
    detail.hidden = true; openId = null; pendingDelete = null;
    restoreRowFocus();
  };
  detail.addEventListener('cancel', event => { event.preventDefault(); closeDetail(); });
  detail.addEventListener('keydown', event => {
    // Escape belongs to this dialog, never the Settings screen behind it.
    event.stopPropagation();
    if (event.key === 'Escape') { event.preventDefault(); closeDetail(); }
  });
  detail.addEventListener('click', event => { if (event.target === detail) {
    const rect = detail.getBoundingClientRect();
    if (event.clientX < rect.left || event.clientX > rect.right || event.clientY < rect.top || event.clientY > rect.bottom) closeDetail();
  } });
  search.addEventListener('input', () => { render(current); });
  types.addEventListener('change', () => { render(current); });
  element.addEventListener('settings-search', event => {
    const query = (event as CustomEvent<string>).detail;
    if (query === globalQuery) return;
    globalQuery = query; render(current);
  });
  list.addEventListener('keydown', event => {
    if (!['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) return;
    const buttons = Array.from(list.querySelectorAll<HTMLButtonElement>('.settings-extension-group, .settings-extension-open')).filter(button => !button.closest('[hidden]'));
    if (!buttons.length) return;
    event.preventDefault();
    const index = buttons.indexOf(document.activeElement as HTMLButtonElement);
    const next = event.key === 'Home' ? 0 : event.key === 'End' ? buttons.length - 1 : Math.max(0, Math.min(buttons.length - 1, index + (event.key === 'ArrowDown' ? 1 : -1)));
    buttons[next]?.focus();
  });

  const messageError = (error: string): void => {
    message.textContent = error;
    if (detail.open) {
      let output = detail.querySelector<HTMLElement>('.settings-extension-message');
      if (!output) {
        output = el('div', 'settings-extension-message'); output.setAttribute('role', 'status'); detail.append(output);
      }
      output.textContent = error;
    }
  };

  const toggleFor = (record: ExtensionRecord): HTMLButtonElement => {
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
      message.textContent = '';
      try {
        const updated = await api.setEnabled({ id: record.id, enabled: next });
        busy.delete(record.id);
        if (!alive) return;
        ++refreshVersion;
        render(updated);
      } catch (error) {
        busy.delete(record.id);
        if (!alive) return;
        render(current);
        messageError(error instanceof Error ? error.message : 'The extension could not be updated.');
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
      if (detail.open) closeDetail();
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
    refreshMetadata(records);
    updateTypeOptions(sorted);
    const query = search.value.trim().toLowerCase();
    const enabled = sorted.filter(record => record.enabled).length;
    summary.textContent = `${sorted.length} installed`;
    const activeFilter = document.activeElement instanceof HTMLElement ? document.activeElement.dataset.extensionFilter : undefined;
    filters.replaceChildren();
    for (const [value, label, count] of [['all', 'All', sorted.length], ['enabled', 'Enabled', enabled], ['off', 'Off', sorted.length - enabled], ['attention', 'Needs attention', sorted.filter(record => ['warning', 'setup'].includes(displayState(record).tone)).length]] as const) {
      const button = el('button', filter === value ? 'btn on' : 'btn', `${label} ${count}`);
      button.type = 'button'; button.dataset.extensionFilter = value;
      button.setAttribute('aria-pressed', String(filter === value));
      button.addEventListener('click', () => { filter = value; render(current); });
      filters.append(button);
      if (activeFilter === value) button.focus({ preventScroll: true });
    }
    // Build copy once per update; scrolling never rebuilds rows. CSS content-visibility
    // skips offscreen layout/paint while preserving native Tab and in-page search.
    const index = new Map(sorted.map(record => [record.id, searchable(record)]));
    const matches = sorted.filter(record => (!query || index.get(record.id)!.includes(query)) && (!globalQuery || index.get(record.id)!.includes(globalQuery))
      && (types.value === 'all' || categoriesOf(record).includes(types.value as ExtensionGroupId))
      && (filter === 'all' || filter === 'enabled' && record.enabled || filter === 'off' && !record.enabled || filter === 'attention' && ['warning', 'setup'].includes(displayState(record).tone)));
    element.dataset.settingsMatchCount = String(sorted.filter(record => !globalQuery || index.get(record.id)!.includes(globalQuery)).length);
    footer.replaceChildren(el('span', '', matches.length === sorted.length ? `${sorted.length} installed` : `${matches.length} of ${sorted.length} extensions`),
      el('span', '', `${enabled} enabled · ${sorted.length - enabled} off`));
    const focused = document.activeElement as HTMLElement;
    const focusedId = focused?.closest<HTMLElement>('.settings-extension-row')?.dataset.extensionId;
    const focusedToggle = focused?.classList.contains('toggle');
    list.replaceChildren();
    if (sorted.length === 0) {
      list.append(el('div', 'settings-extension-empty', 'No extensions installed.'));
      return;
    }
    if (!matches.length) {
      const empty = el('div', 'settings-extension-empty', 'No extensions match your search or filters.');
      const reset = el('button', 'btn', 'Clear filters'); reset.type = 'button';
      reset.addEventListener('click', () => { search.value = ''; types.value = 'all'; filter = 'all'; render(current); search.focus(); });
      empty.append(reset); list.append(empty); return;
    }
    const resolveName = (id: string): string => nameOf(records.find(item => item.id === id) ?? { id } as ExtensionRecord);
    for (const { id: kind, label } of EXTENSION_GROUPS) {
      const members = matches.filter(record => category(record) === kind);
      if (!members.length) continue;
      const group = el('div', 'settings-extension-category');
      const heading = el('button', 'settings-extension-group');
      const groupIcon = el('i', 'settings-extension-group-icon');
      groupIcon.setAttribute('aria-hidden', 'true');
      const glyph = icon(GROUP_ICONS[kind]); if (glyph) groupIcon.append(glyph);
      const count = el('span', 'settings-extension-count', String(members.length));
      count.setAttribute('aria-hidden', 'true');
      heading.append(groupIcon, el('span', 'settings-extension-group-label', label), count);
      heading.setAttribute('aria-label', `${label}, ${members.length} ${members.length === 1 ? 'extension' : 'extensions'}`);
      heading.type = 'button'; heading.dataset.category = kind;
      const isCollapsed = collapsed.has(kind) && !query && !globalQuery;
      heading.setAttribute('aria-expanded', String(!isCollapsed));
      const body = el('div', 'settings-extension-group-body'); body.hidden = isCollapsed;
      body.id = `${controlId}-${kind}`; heading.setAttribute('aria-controls', body.id);
      heading.addEventListener('click', () => {
        if (body.hidden) collapsed.delete(kind); else collapsed.add(kind);
        body.hidden = !body.hidden; heading.setAttribute('aria-expanded', String(!body.hidden));
      });
      group.append(heading, body); list.append(group);
      const nested = kind === 'effects' || (kind === 'panels' || kind === 'appearance')
        && members.some(record => subcategoryTerms(record, kind).length > 0);
      const destinations = new Map<string, HTMLElement>();
      if (nested) {
        const labels = unique(members.map(record => subcategoryOf(record, kind))).sort((a, b) => {
          if (kind === 'effects') return (effectOrder.get(a) ?? 10000) - (effectOrder.get(b) ?? 10000) || a.localeCompare(b);
          return a.localeCompare(b);
        });
        for (const [index, label] of labels.entries()) {
          const entries = members.filter(record => subcategoryOf(record, kind) === label);
          const key = `${kind}\0${label}`;
          const section = el('div', 'settings-extension-subcategory');
          const disclosure = el('button', 'settings-extension-group is-subgroup');
          const count = el('span', 'settings-extension-count', String(entries.length));
          count.setAttribute('aria-hidden', 'true');
          disclosure.append(el('span', 'settings-extension-group-label', label), count);
          disclosure.setAttribute('aria-label', `${label}, ${entries.length} ${entries.length === 1 ? 'extension' : 'extensions'}`);
          disclosure.type = 'button'; disclosure.dataset.subcategory = label; disclosure.dataset.parentCategory = kind;
          const content = el('div', 'settings-extension-subgroup-body');
          content.id = `${controlId}-${kind}-sub-${index}`;
          content.hidden = !expandedSubcategories.has(key) && !query && !globalQuery;
          disclosure.setAttribute('aria-controls', content.id); disclosure.setAttribute('aria-expanded', String(!content.hidden));
          disclosure.addEventListener('click', () => {
            if (content.hidden) expandedSubcategories.add(key); else expandedSubcategories.delete(key);
            content.hidden = !content.hidden; disclosure.setAttribute('aria-expanded', String(!content.hidden));
          });
          section.append(disclosure, content); body.append(section); destinations.set(label, content);
        }
      }
      for (const record of members) {
        const state = displayState(record);
        if (record.health.state === 'replaced') state.detail = `Replaced by ${resolveName(record.health.by)}`;
        const row = el('div', 'settings-extension-row');
        row.setAttribute('role', 'listitem');
        row.dataset.extensionId = record.id;
        row.title = `${record.id}${record.manifest?.version ? ` · v${record.manifest.version}` : ''}`;

        const open = el('button', 'settings-extension-open');
        open.type = 'button';
        open.setAttribute('aria-label', `Open ${nameOf(record)}`);
        open.setAttribute('aria-haspopup', 'dialog');
        const categories = categoriesOf(record);
        open.setAttribute('aria-description', `Provides ${categories.map(group => categoryNames[group]).join(', ')}.`);
        const copy = el('div', 'settings-extension-copy');
        const heading = el('div', 'settings-extension-name');
        heading.append(dotFor(state.tone), el('b', '', nameOf(record)));
        if (record.scope !== 'builtin' && record.manifest?.author !== 'agent') heading.append(el('span', 'settings-extension-tag', sourceLabel(record)));
        const secondary = categories.filter(group => group !== category(record) && group !== 'tools');
        const families = subcategoryTerms(record, category(record));
        const hints = [...secondary.map(group => categoryNames[group]), ...(families.length > 1 ? families : [])];
        if (hints.length) heading.append(el('span', 'settings-extension-kinds', hints.join(' · ')));
        if (families.length) open.setAttribute('aria-description', `${open.getAttribute('aria-description')} Categories: ${families.join(', ')}.`);
        copy.append(heading);
        const hint = state.tone === 'setup' ? state.label : state.detail ?? record.manifest?.description ?? '';
        if (hint) {
          const line = el('span', state.tone === 'warning' ? 'settings-extension-error' : state.tone === 'setup' ? 'settings-extension-setup-note' : '', hint);
          line.title = hint;
          copy.append(line);
        }
        open.append(copy);

        const controls = el('div', 'settings-extension-controls');
        const status = el('span', `settings-extension-status is-${state.tone}`, state.label);
        controls.append(status);
        if (state.tone === 'setup') controls.append(setupButton(record, 'Set up'));
        controls.append(toggleFor(record));
        const chevron = el('i', 'settings-extension-chevron');
        chevron.setAttribute('aria-hidden', 'true');
        const glyph = icon('chev');
        if (glyph) chevron.append(glyph);
        controls.append(chevron);

        const show = (): void => {
          message.textContent = '';
          openId = returnId = record.id; pendingDelete = null; renderDetail(record, current);
          detail.hidden = false; detail.showModal();
        };
        open.addEventListener('click', show);
        row.addEventListener('click', (event) => {
          if ((event.target as HTMLElement).closest('.toggle, .settings-extension-open, .settings-extension-setup')) return;
          show();
        });
        row.append(open, controls);
        (destinations.get(subcategoryOf(record, kind)) ?? body).append(row);
      }
    }
    if (focusedId) {
      const row = Array.from(list.querySelectorAll<HTMLElement>('[data-extension-id]')).find(row => row.dataset.extensionId === focusedId);
      (row?.querySelector<HTMLButtonElement>(focusedToggle ? '.toggle' : '.settings-extension-open') ?? search).focus({ preventScroll: true });
    }
  };

  const renderDetail = (record: ExtensionRecord, records: ExtensionRecord[]): void => {
    const resolveName = (id: string): string => nameOf(records.find(item => item.id === id) ?? { id } as ExtensionRecord);
    const state = displayState(record);
    if (record.health.state === 'replaced') state.detail = `Replaced by ${resolveName(record.health.by)}`;
    detail.replaceChildren();
    detail.dataset.extensionId = record.id;

    const back = el('button', 'btn settings-extension-back', 'Done');
    back.type = 'button'; back.setAttribute('aria-label', 'Close extension details');
    back.addEventListener('click', closeDetail);

    const head = el('header', 'settings-extension-head');
    const title = el('div', 'settings-extension-title');
    const titleText = el('h3', '', nameOf(record));
    titleText.id = `${controlId}-title`; detail.setAttribute('aria-labelledby', titleText.id);
    title.append(dotFor(state.tone), titleText);
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
    stateControls.append(toggleFor(record));
    stateRow.append(stateCopy, stateControls);
    status.body.append(stateRow);

    const about = createSettingsSection('About');
    const version = record.manifest?.version ? ` · v${record.manifest.version}` : '';
    about.body.append(rowOf('Identifier', `${record.id}${version}`));
    const replaces = record.manifest?.replaces ?? [];
    if (replaces.length) about.body.append(rowOf(record.enabled && state.label === 'Active' ? 'Replaces' : 'When active, replaces', replaces.map(resolveName).join(', '), 'replacement'));
    for (const [kind, label] of [['effects', 'Effect categories'], ['panels', 'Panel categories'], ['appearance', 'Appearance']] as const) {
      const terms = subcategoryTerms(record, kind);
      if (terms.length) about.body.append(rowOf(label, terms.join(' · ')));
    }
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
            busy.delete(record.id);
            if (!alive) return;
            ++refreshVersion; pendingDelete = null; closeDetail(); render(updated); restoreRowFocus();
          } catch (error) {
            busy.delete(record.id); if (!alive) return; render(current);
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
    current = records;
    const sorted = records.filter(record => options.includeBuiltin || record.scope !== 'builtin').sort(compareRecords);
    const open = openId ? sorted.find(record => record.id === openId) ?? null : null;
    if (openId && !open) closeDetail();
    renderList(records, sorted);
    detail.hidden = !open;
    if (open) {
      const focused = document.activeElement as HTMLElement;
      const label = detail.contains(focused) ? focused.getAttribute('aria-label') : null;
      const wasToggle = detail.contains(focused) && focused.classList.contains('toggle');
      renderDetail(open, records);
      if (label) Array.from(detail.querySelectorAll<HTMLButtonElement>('button')).find(button => button.getAttribute('aria-label') === label || wasToggle && button.classList.contains('toggle'))?.focus({ preventScroll: true });
    }
    else { detail.replaceChildren(); delete detail.dataset.extensionId; }
  };

  const refresh = async (): Promise<void> => {
    const version = ++refreshVersion;
    if (!api) {
      message.textContent = 'Restart Powermove to manage extensions.';
      list.replaceChildren(el('div', 'settings-extension-empty', 'Extensions are unavailable in this session.'));
      return;
    }
    try {
      const records = await api.list();
      if (version === refreshVersion) render(records);
    } catch (error) {
      if (!alive) return;
      message.textContent = error instanceof Error ? error.message : 'Extensions could not be loaded.';
      list.replaceChildren();
    }
  };

  let metadataQueued = false;
  const registryStops = [kernel?.effects, kernel?.commands, kernel?.themes].map(registry => registry?.onChange?.(() => {
    if (metadataQueued || !alive) return;
    metadataQueued = true;
    queueMicrotask(() => { metadataQueued = false; if (alive) render(current); });
  })).filter(Boolean);
  if (api) {
    stop = api.onChanged((_event: ExtensionsChangedEvent) => void refresh());
  }
  void refresh();

  return {
    element,
    destroy: () => {
      alive = false;
      if (detail.open) detail.close();
      detail.remove();
      stop();
      for (const subscription of registryStops) subscription.dispose();
    }
  };
}

export const extensionSettingsLabels = { displayState, sourceLabel };
