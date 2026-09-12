// @vitest-environment happy-dom
import { describe, expect, it, vi } from 'vitest';

import { createGeneralSettingsControl } from './general-settings';

describe('general settings control', () => {
  it('groups accounts and application preferences into labelled sections', () => {
    const control = createGeneralSettingsControl({ mode: 'system', apply: vi.fn() });
    const headings = [...control.element.querySelectorAll('.settings-section-heading > b')]
      .map((node) => node.textContent);
    expect(headings).toEqual(['Accounts', 'Application']);
    const accounts = control.element.querySelector('.settings-section-body');
    expect(accounts?.querySelectorAll('.settings-provider')).toHaveLength(3);
    control.destroy();
  });

  it('seeds the appearance select from the theme and applies changes', () => {
    const apply = vi.fn();
    const control = createGeneralSettingsControl({ mode: 'dark', apply });
    const select = control.element.querySelector<HTMLSelectElement>('.settings-appearance')!;
    expect(select.value).toBe('dark');
    expect([...select.options].map((option) => option.value)).toEqual(['system', 'light', 'dark']);
    select.value = 'light';
    select.dispatchEvent(new Event('change'));
    expect(apply).toHaveBeenCalledWith('light');
    control.destroy();
  });
});
