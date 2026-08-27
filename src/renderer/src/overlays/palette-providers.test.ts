import { describe, expect, it, vi } from 'vitest';

import { createKernel } from '../kernel/registries';
import { paletteEntries } from './palette-model';

function model(kernel = createKernel()) {
  return {
    Kernel: kernel,
    commands: {
      undo: { id: 'undo', label: 'Undo', cat: 'Edit', run: () => {} },
      hidden: { id: 'hidden', label: 'Hidden', cat: 'Edit', when: () => false, run: () => {} }
    },
    cmd: vi.fn(),
    proj: { layers: [] },
    WS: { list: () => [] },
    FX: {},
    kernel
  } as any;
}

describe('palette entries with kernel contributions', () => {
  it('skips commands whose when() is false', () => {
    const entries = paletteEntries(model(), '');
    expect(entries.map((entry) => entry.id)).toEqual(['command:undo']);
  });

  it('appends provider entries after the legacy groups', () => {
    const PM = model();
    PM.Kernel.registerPaletteProvider('ext:notes', () => [
      { id: 'note:1', label: 'Open scratchpad', category: 'Notes', kb: '⌘J', run: () => 'opened' }
    ]);

    const entries = paletteEntries(PM, '');

    expect(entries.map((entry) => entry.id)).toEqual(['command:undo', 'note:1']);
    expect(entries[1]).toMatchObject({ label: 'Open scratchpad', cat: 'Notes', kb: '⌘J' });
    expect(entries[1]!.run()).toBe('opened');
  });

  it('passes the raw query through to the provider', () => {
    const PM = model();
    const provider = vi.fn(() => []);
    PM.Kernel.registerPaletteProvider('ext:notes', provider);

    paletteEntries(PM, '  Note ');

    expect(provider).toHaveBeenCalledWith('  Note ');
  });

  it('merges several providers in registration order', () => {
    const PM = model();
    PM.Kernel.registerPaletteProvider('a', () => [{ id: 'a:1', label: 'A', category: 'A', run: () => {} }]);
    PM.Kernel.registerPaletteProvider('b', () => [{ id: 'b:1', label: 'B', category: 'B', run: () => {} }]);

    expect(paletteEntries(PM, '').map((entry) => entry.id)).toEqual(['command:undo', 'a:1', 'b:1']);
  });

  it('ignores a provider that throws or returns something that is not a list', () => {
    const PM = model();
    const error = vi.spyOn(console, 'error').mockImplementation(() => {});
    PM.Kernel.registerPaletteProvider('bad', () => { throw new Error('nope'); });
    PM.Kernel.registerPaletteProvider('worse', (() => null) as never);

    expect(paletteEntries(PM, '').map((entry) => entry.id)).toEqual(['command:undo']);
    error.mockRestore();
  });

  it('keeps provider entries inside the 60-item cap', () => {
    const PM = model();
    PM.commands = Object.fromEntries(
      Array.from({ length: 59 }, (_, index) => [`c${index}`, { id: `c${index}`, label: `C${index}`, cat: 'Command', run: () => {} }])
    );
    PM.Kernel.registerPaletteProvider('many', () => Array.from({ length: 10 }, (_, index) => ({ id: `p${index}`, label: `P${index}`, category: 'Ext', run: () => {} })));

    const entries = paletteEntries(PM, '');

    expect(entries).toHaveLength(60);
    expect(entries.filter((entry) => entry.cat === 'Ext')).toHaveLength(1);
  });

  it('drops provider entries with no run function', () => {
    const PM = model();
    PM.Kernel.registerPaletteProvider('sloppy', () => [{ id: 'x', label: 'X' }] as never);

    expect(paletteEntries(PM, '').map((entry) => entry.id)).toEqual(['command:undo']);
  });
});
