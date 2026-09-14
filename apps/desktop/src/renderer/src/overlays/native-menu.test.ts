import { describe, expect, it } from 'vitest';

import { acceleratorFor, canRenderNatively, planNativeMenu } from './native-menu';

describe('acceleratorFor', () => {
  it('maps glyph shortcuts to Electron accelerators', () => {
    expect(acceleratorFor('⌘Z')).toBe('CommandOrControl+Z');
    expect(acceleratorFor('⌘⇧G')).toBe('CommandOrControl+Shift+G');
    expect(acceleratorFor('⌥⌘R')).toBe('Alt+CommandOrControl+R');
    expect(acceleratorFor('⌫')).toBe('Backspace');
    expect(acceleratorFor('F2')).toBe('F2');
    expect(acceleratorFor('M')).toBe('M');
    expect(acceleratorFor('⌘1')).toBe('CommandOrControl+1');
  });
  it('drops shortcuts it cannot express', () => {
    expect(acceleratorFor(null)).toBeUndefined();
    expect(acceleratorFor('')).toBeUndefined();
    expect(acceleratorFor('⌘')).toBeUndefined();
    expect(acceleratorFor('Click')).toBeUndefined();
  });
});

describe('planNativeMenu', () => {
  it('turns rows into native items and keeps the actions by id', () => {
    const run = () => undefined;
    const plan = planNativeMenu([
      { header: 'Layer' },
      { label: 'Rename', kb: 'F2', run },
      '-',
      { label: 'Shy', on: true, run },
      { label: 'Lock', on: false, disabled: true, run }
    ]);
    expect(plan.items).toEqual([
      { type: 'header', id: 'm0', label: 'Layer' },
      { type: 'normal', id: 'm1', label: 'Rename', enabled: true, checked: false, accelerator: 'F2' },
      { type: 'separator' },
      { type: 'checkbox', id: 'm3', label: 'Shy', enabled: true, checked: true, accelerator: undefined },
      { type: 'checkbox', id: 'm4', label: 'Lock', enabled: false, checked: false, accelerator: undefined }
    ]);
    expect([...plan.actions.keys()]).toEqual(['m1', 'm3', 'm4']);
  });
});

describe('canRenderNatively', () => {
  it('keeps curve pickers and empty menus in the DOM', () => {
    expect(canRenderNatively([{ label: 'A' }])).toBe(true);
    expect(canRenderNatively([{ header: 'Only a title' }, '-'])).toBe(false);
    expect(canRenderNatively([{ label: 'Ease', curve: [0.4, 0, 0.2, 1] }])).toBe(false);
  });
});
