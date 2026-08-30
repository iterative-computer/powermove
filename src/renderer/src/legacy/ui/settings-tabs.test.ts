// @vitest-environment happy-dom
import { beforeEach, describe, expect, it } from 'vitest';

import { createSettingsTabs } from './settings-tabs';

describe('Settings tabs', () => {
  beforeEach(() => document.body.replaceChildren());

  it('shows one labelled panel at a time and switches on click', () => {
    const general = document.createElement('section');
    general.textContent = 'Account';
    const extensions = document.createElement('section');
    extensions.textContent = 'Extension list';
    const tabs = createSettingsTabs([
      { id: 'general', label: 'General', panel: general },
      { id: 'extensions', label: 'Extensions', panel: extensions }
    ]);
    document.body.append(tabs.element);

    const [generalTab, extensionsTab] = [...tabs.element.querySelectorAll<HTMLButtonElement>('[role="tab"]')];
    expect(generalTab?.getAttribute('aria-selected')).toBe('true');
    expect(general.hidden).toBe(false);
    expect(extensions.hidden).toBe(true);

    extensionsTab?.click();
    expect(extensionsTab?.getAttribute('aria-selected')).toBe('true');
    expect(general.hidden).toBe(true);
    expect(extensions.hidden).toBe(false);
    expect(extensions.getAttribute('aria-labelledby')).toBe(extensionsTab?.id);
  });

  it('supports arrow, Home, End, and an initial tab', () => {
    const panels = ['general', 'extensions', 'advanced'].map(() => document.createElement('section'));
    const tabs = createSettingsTabs([
      { id: 'general', label: 'General', panel: panels[0]! },
      { id: 'extensions', label: 'Extensions', panel: panels[1]! },
      { id: 'advanced', label: 'Advanced', panel: panels[2]! }
    ], 'extensions');
    document.body.append(tabs.element);
    const buttons = [...tabs.element.querySelectorAll<HTMLButtonElement>('[role="tab"]')];

    expect(buttons[1]?.tabIndex).toBe(0);
    buttons[1]?.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true }));
    expect(buttons[2]?.getAttribute('aria-selected')).toBe('true');
    expect(document.activeElement).toBe(buttons[2]);
    buttons[2]?.dispatchEvent(new KeyboardEvent('keydown', { key: 'Home', bubbles: true }));
    expect(buttons[0]?.getAttribute('aria-selected')).toBe('true');
    buttons[0]?.dispatchEvent(new KeyboardEvent('keydown', { key: 'End', bubbles: true }));
    expect(buttons[2]?.getAttribute('aria-selected')).toBe('true');
  });
});
