import { describe, expect, it, vi } from 'vitest';

import type { KeybindingDefinition, PowermoveAPI } from 'powermove';

import activate from './index';
import { KEYMAP_DEFAULT } from './keymap';

describe('keymap-default', () => {
  it('registers the complete built-in keymap with its intended priorities', () => {
    const captured: KeybindingDefinition[] = [];
    const bind = vi.fn((definition: KeybindingDefinition) => {
      captured.push(definition);
      return { dispose() {} };
    });

    activate({ keybindings: { bind } } as unknown as PowermoveAPI);

    expect(captured).toEqual(KEYMAP_DEFAULT);
    expect(captured).toHaveLength(138);
    expect(captured.filter(({ command }) => command !== 'blurField')).toHaveLength(137);
    expect(captured.find(({ command }) => command === 'blurField')).toEqual({
      key: 'escape',
      command: 'blurField',
      inFields: true,
      priority: 50
    });
    expect(captured.filter(({ command }) => command !== 'blurField').every(({ priority }) => priority === 100)).toBe(true);
  });
});
