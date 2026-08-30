import { createChatGPTSettingsControl } from './chatgpt-settings';
import { createSettingsSection } from './settings-tabs';

/** The slice of PM.theme the Settings modal needs. */
export interface ThemePreference {
  readonly mode: string;
  apply(mode: string): void;
}

export interface GeneralSettingsControl {
  element: HTMLElement;
  focus(): void;
  destroy(): void;
}

const APPEARANCE_OPTIONS: Array<[value: string, label: string]> = [
  ['system', 'Match system'],
  ['light', 'Light'],
  ['dark', 'Dark']
];

function createRow(title: string, detail: string, control: HTMLElement): HTMLElement {
  const row = document.createElement('div');
  row.className = 'settings-row';
  const copy = document.createElement('div');
  copy.className = 'settings-copy';
  const label = document.createElement('b');
  label.textContent = title;
  const description = document.createElement('span');
  description.textContent = detail;
  copy.append(label, description);
  row.append(copy, control);
  return row;
}

/** The General tab: subscription accounts on top, app preferences below. */
export function createGeneralSettingsControl(theme: ThemePreference): GeneralSettingsControl {
  const element = document.createElement('div');
  element.className = 'settings-sections';

  const chatgpt = createChatGPTSettingsControl();
  const accounts = createSettingsSection(
    'Accounts',
    'Signed in through the official Codex runtime'
  );
  accounts.body.append(chatgpt.element);
  const privacy = document.createElement('p');
  privacy.className = 'settings-note';
  privacy.textContent = 'Powermove never reads or stores your account token.';
  accounts.element.append(privacy);

  const appearance = document.createElement('select');
  appearance.className = 'settings-appearance';
  appearance.setAttribute('aria-label', 'Appearance');
  for (const [value, label] of APPEARANCE_OPTIONS) {
    const option = document.createElement('option');
    option.value = value;
    option.textContent = label;
    appearance.append(option);
  }
  appearance.value = theme.mode;
  appearance.addEventListener('change', () => theme.apply(appearance.value));
  const application = createSettingsSection('Application');
  application.body.append(
    createRow('Appearance', 'Follow your system setting, or pick light or dark.', appearance)
  );

  element.append(accounts.element, application.element);

  return {
    element,
    focus: () => chatgpt.focus(),
    destroy: () => chatgpt.destroy()
  };
}
