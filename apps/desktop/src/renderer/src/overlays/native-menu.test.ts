import { describe, expect, it, vi } from 'vitest';

import { acceleratorFor, canRenderNatively, iconRasterizer, planNativeMenu } from './native-menu';

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
  it('turns rows into native items, rasterizes icons, and keeps the actions by id', async () => {
    const run = () => undefined;
    const icon = vi.fn(async (name: string) => (name === 'trash' ? 'data:image/png;base64,AAAA' : undefined));
    const plan = await planNativeMenu([
      { header: 'Layer' },
      { label: 'Rename', kb: 'F2', run },
      '-',
      { label: 'Shy', on: true, run },
      { label: 'Lock', on: false, disabled: true, run },
      { label: 'Delete', icon: 'trash', kb: '⌫', run },
      { label: 'Odd', icon: 'nope', run }
    ], icon);
    expect(plan.items).toEqual([
      { type: 'header', id: 'm0', label: 'Layer' },
      { type: 'normal', id: 'm1', label: 'Rename', enabled: true, checked: false, accelerator: 'F2', icon: undefined },
      { type: 'separator' },
      { type: 'checkbox', id: 'm3', label: 'Shy', enabled: true, checked: true, accelerator: undefined, icon: undefined },
      { type: 'checkbox', id: 'm4', label: 'Lock', enabled: false, checked: false, accelerator: undefined, icon: undefined },
      { type: 'normal', id: 'm5', label: 'Delete', enabled: true, checked: false, accelerator: 'Backspace', icon: 'data:image/png;base64,AAAA' },
      { type: 'normal', id: 'm6', label: 'Odd', enabled: true, checked: false, accelerator: undefined, icon: undefined }
    ]);
    expect([...plan.actions.keys()]).toEqual(['m1', 'm3', 'm4', 'm5', 'm6']);
    expect(icon).toHaveBeenCalledTimes(2);
  });

  it('survives a rasterizer that throws', async () => {
    const plan = await planNativeMenu([{ label: 'A', icon: 'x' }], async () => { throw new Error('no canvas'); });
    expect(plan.items[0]).toMatchObject({ label: 'A', icon: undefined });
  });
});

describe('iconRasterizer', () => {
  it('skips unknown icons without touching the DOM', async () => {
    const rasterize = iconRasterizer({ known: '<path d="M0 0h1v1z"/>' });
    expect(await rasterize('missing')).toBeUndefined();
  });
});

describe('canRenderNatively', () => {
  it('keeps curve pickers and empty menus in the DOM', () => {
    expect(canRenderNatively([{ label: 'A' }])).toBe(true);
    expect(canRenderNatively([{ header: 'Only a title' }, '-'])).toBe(false);
    expect(canRenderNatively([{ label: 'Ease', curve: [0.4, 0, 0.2, 1] }])).toBe(false);
  });
});
