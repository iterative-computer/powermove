export interface SettingsTabDefinition {
  id: string;
  label: string;
  panel: HTMLElement;
}

export interface SettingsTabsControl {
  element: HTMLElement;
  select(id: string, focus?: boolean): void;
}

let settingsTabsInstance = 0;

/** Small, accessible tab controller shared by the Settings modal. */
export function createSettingsTabs(
  definitions: SettingsTabDefinition[],
  initialId = definitions[0]?.id
): SettingsTabsControl {
  if (definitions.length === 0) throw new Error('Settings needs at least one tab.');
  const instance = ++settingsTabsInstance;
  const element = document.createElement('div');
  element.className = 'settings-tabs-layout';
  const tablist = document.createElement('div');
  tablist.className = 'settings-tabs';
  tablist.setAttribute('role', 'tablist');
  tablist.setAttribute('aria-label', 'Settings sections');
  const pages = document.createElement('div');
  pages.className = 'settings-pages';
  const buttons = new Map<string, HTMLButtonElement>();
  const panels = new Map<string, HTMLElement>();

  const select = (id: string, focus = false): void => {
    const selected = buttons.has(id) ? id : definitions[0]!.id;
    for (const definition of definitions) {
      const active = definition.id === selected;
      const button = buttons.get(definition.id)!;
      button.setAttribute('aria-selected', String(active));
      button.tabIndex = active ? 0 : -1;
      const panel = panels.get(definition.id)!;
      panel.hidden = !active;
      if (active && focus) button.focus();
    }
  };

  definitions.forEach((definition, index) => {
    const tabId = `settings-tab-${instance}-${definition.id}`;
    const panelId = `settings-panel-${instance}-${definition.id}`;
    const button = document.createElement('button');
    button.type = 'button';
    button.className = 'settings-tab';
    button.id = tabId;
    button.textContent = definition.label;
    button.setAttribute('role', 'tab');
    button.setAttribute('aria-controls', panelId);
    button.addEventListener('click', () => select(definition.id));
    button.addEventListener('keydown', (event) => {
      const keys = ['ArrowLeft', 'ArrowRight', 'Home', 'End'];
      if (!keys.includes(event.key)) return;
      event.preventDefault();
      const current = definitions.findIndex((candidate) => candidate.id === definition.id);
      const next = event.key === 'Home'
        ? 0
        : event.key === 'End'
          ? definitions.length - 1
          : (current + (event.key === 'ArrowRight' ? 1 : -1) + definitions.length) % definitions.length;
      select(definitions[next]!.id, true);
    });
    buttons.set(definition.id, button);
    tablist.append(button);

    definition.panel.classList.add('settings-page');
    definition.panel.id = panelId;
    definition.panel.setAttribute('role', 'tabpanel');
    definition.panel.setAttribute('aria-labelledby', tabId);
    definition.panel.tabIndex = 0;
    panels.set(definition.id, definition.panel);
    pages.append(definition.panel);
  });

  element.append(tablist, pages);
  select(initialId ?? definitions[0]!.id);
  return { element, select };
}
