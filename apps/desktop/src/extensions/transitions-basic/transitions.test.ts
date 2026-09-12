import { describe, expect, it, vi } from 'vitest';

import type { PowermoveAPI, TransitionDefinition } from 'powermove';

import activate from './index';

describe('transitions-basic', () => {
  it('registers the four built-in transition ids', () => {
    const registered: TransitionDefinition[] = [];
    const register = vi.fn((definition: TransitionDefinition) => {
      registered.push(definition);
      return { dispose() {} };
    });
    const api = { transitions: { register } } as unknown as PowermoveAPI;

    activate(api);

    expect(registered.map((definition) => definition.id)).toEqual(['crossfade', 'wipe', 'slide', 'zoom']);
    expect(register).toHaveBeenCalledTimes(4);
  });
});
