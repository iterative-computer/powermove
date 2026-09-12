import { describe, expect, it } from 'vitest';

import { BUILTIN_EXTENSIONS } from './builtins';

describe('built-in extension order', () => {
  it('loads foundations, core surfaces, then Mods in the documented layout order', () => {
    expect(Object.keys(BUILTIN_EXTENSIONS)).toEqual([
      'theme-default',
      'keymap-default',
      'effects-basic',
      'transitions-basic',
      'layers-3d',
      'toolbar',
      'viewer',
      'timeline',
      'inspector',
      'mods'
    ]);
  });
});
