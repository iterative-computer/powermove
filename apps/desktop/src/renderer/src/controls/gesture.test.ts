import { describe, expect, it, vi } from 'vitest';

import { EditGesture } from './gesture';

function fakePM() {
  const Edit = { begin: vi.fn(), dispatch: vi.fn(), commit: vi.fn(), cancel: vi.fn(), apply: vi.fn(() => ({ ok: true })) };
  const hist = { begin: vi.fn(), commit: vi.fn(), cancel: vi.fn(), do: vi.fn((_l: string, fn: () => void) => fn()) };
  return { Edit, hist };
}

describe('EditGesture', () => {
  it('command mode drives a typed Edit transaction with the label and origin', () => {
    const PM = fakePM();
    const g = new EditGesture(PM, {
      mode: 'command', label: 'Opacity', origin: 'inspector',
      command: { type: 'set_property', target: 'L1', path: 'opacity', value: 0, mode: 'auto' } as any
    });
    g.begin();
    g.write(50);
    g.commit();
    expect(PM.Edit.begin).toHaveBeenCalledWith('Opacity', { origin: 'inspector' });
    expect(PM.Edit.dispatch).toHaveBeenCalledWith(expect.objectContaining({ type: 'set_property', value: 50 }));
    expect(PM.Edit.commit).toHaveBeenCalledWith('Opacity');
  });

  it('click-without-drag still begins and cancels (Edit.cancel fast path)', () => {
    const PM = fakePM();
    const g = new EditGesture(PM, { mode: 'command', label: 'X', command: (v: unknown) => ({ type: 'set_layer', target: 'L1', patch: { name: v } } as any) });
    g.begin();
    g.cancel();
    expect(PM.Edit.dispatch).not.toHaveBeenCalled();
    expect(PM.Edit.cancel).toHaveBeenCalledTimes(1);
  });

  it('once() applies one-shot with the default interface origin', () => {
    const PM = fakePM();
    const g = new EditGesture(PM, { mode: 'command', label: 'Toggle', command: { type: 'set_layer', target: 'L1', patch: {} } as any });
    g.once(true);
    expect(PM.Edit.apply).toHaveBeenCalledWith(expect.objectContaining({ value: true }), { label: 'Toggle', origin: 'interface' });
  });

  it('local mode never touches Edit or history', () => {
    const PM = fakePM();
    const set = vi.fn();
    const g = new EditGesture(PM, { mode: 'local', label: 'Tool', set });
    g.begin(); g.write(1); g.commit(); g.once(2);
    expect(set).toHaveBeenCalledTimes(2);
    expect(PM.Edit.begin).not.toHaveBeenCalled();
    expect(PM.hist.begin).not.toHaveBeenCalled();
  });

  it('legacy set mode wraps in history', () => {
    const PM = fakePM();
    const set = vi.fn();
    const g = new EditGesture(PM, { mode: 'set', label: 'Raw', set });
    g.begin(); g.write(3); g.commit();
    expect(PM.hist.begin).toHaveBeenCalledWith('Raw');
    expect(PM.hist.commit).toHaveBeenCalledWith('Raw');
    g.once(4);
    expect(PM.hist.do).toHaveBeenCalled();
    expect(set).toHaveBeenLastCalledWith(4);
  });
});
