import { describe, expect, it } from 'vitest';

import { contextMenuItems, tidyMenu } from './menu-rules';

describe('tidyMenu', () => {
  it('keeps separators only between groups and drops titles with nothing under them', () => {
    expect(tidyMenu(['-', { header: 'Empty' }, '-', '-', { label: 'A' }, '-', '-', { header: 'B' }, { label: 'B1' }, { header: 'Dangling' }, '-'])).toEqual([
      { label: 'A' }, '-', { header: 'B' }, { label: 'B1' }
    ]);
  });

  it('gives a group icons on every row or on none', () => {
    expect(tidyMenu([{ label: 'Copy', icon: 'copy' }, { label: 'Odd' }, '-', { label: 'Lock', icon: 'lock' }, { label: 'Hide', icon: 'eye' }])).toEqual([
      { label: 'Copy', icon: undefined }, { label: 'Odd' }, '-', { label: 'Lock', icon: 'lock' }, { label: 'Hide', icon: 'eye' }
    ]);
  });
});

describe('contextMenuItems', () => {
  it('hides unavailable rows, except Cut, Copy and Paste, and drops shortcuts', () => {
    expect(contextMenuItems([
      { label: 'Duplicate', kb: '⌘D' },
      { label: 'Simplify', disabled: true },
      { label: 'Paste', disabled: true, kb: '⌘V' },
      '-',
      { label: 'Delete', disabled: true }
    ])).toEqual([{ label: 'Duplicate' }, { label: 'Paste', disabled: true }]);
  });

  it('drops submenus left with nothing to run', () => {
    expect(contextMenuItems([
      { label: 'Timing', submenu: [{ label: 'Split', disabled: true }] },
      { label: 'Move', submenu: [{ header: 'Groups' }, { label: 'Hero', kb: '⌘1' }, { label: 'Off', disabled: true }] }
    ])).toEqual([{ label: 'Move', submenu: [{ header: 'Groups' }, { label: 'Hero' }] }]);
  });
});
