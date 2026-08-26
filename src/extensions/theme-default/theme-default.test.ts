import { describe, expect, it, vi } from 'vitest';

import type { PowermoveAPI, ThemeDefinition } from 'powermove';

import activate, { THEMES } from './index';

describe('theme-default', () => {
  it('registers the default and high-contrast themes', () => {
    const registered: ThemeDefinition[] = [];
    const api = {
      theme: { register: vi.fn((definition: ThemeDefinition) => void registered.push(definition)) }
    } as unknown as PowermoveAPI;

    activate(api);

    expect(registered.map((theme) => theme.id)).toEqual(['default', 'high-contrast']);
    expect(THEMES[0]).toMatchObject({ name: 'Powermove', scheme: 'auto', tokens: {} });
    expect(Object.keys(THEMES[1]!.tokens ?? {})).toHaveLength(8);
  });
});
