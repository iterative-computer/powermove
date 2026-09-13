import { describe, expect, it, vi } from 'vitest';

import { EditGesture } from './gesture';

function fakeAPI() {
  const edit = {
    begin: vi.fn(),
    dispatch: vi.fn(),
    commit: vi.fn(),
    cancel: vi.fn(),
    apply: vi.fn(() => ({ ok: true }))
  };
  const history = {
    begin: vi.fn(),
    commit: vi.fn(),
    cancel: vi.fn(),
    do: vi.fn((_label: string, operation: () => void) => operation())
  };
  return { edit, history };
}

describe('EditGesture', () => {
  it('command mode drives a typed edit transaction with the label and origin', () => {
    const api = fakeAPI();
    const gesture = new EditGesture(api as any, {
      mode: 'command',
      label: 'Opacity',
      origin: 'inspector',
      command: { type: 'set_property', target: 'L1', path: 'opacity', value: 0, mode: 'auto' } as any
    });
    gesture.begin();
    gesture.write(50);
    gesture.commit();
    expect(api.edit.begin).toHaveBeenCalledWith('Opacity', { origin: 'inspector' });
    expect(api.edit.dispatch).toHaveBeenCalledWith(expect.objectContaining({ type: 'set_property', value: 50 }));
    expect(api.edit.commit).toHaveBeenCalledWith('Opacity');
  });

  it('click-without-drag still begins and cancels (edit.cancel fast path)', () => {
    const api = fakeAPI();
    const gesture = new EditGesture(api as any, {
      mode: 'command',
      label: 'X',
      command: (value: unknown) => ({ type: 'set_layer', target: 'L1', patch: { name: value } } as any)
    });
    gesture.begin();
    gesture.cancel();
    expect(api.edit.dispatch).not.toHaveBeenCalled();
    expect(api.edit.cancel).toHaveBeenCalledTimes(1);
  });

  it('once() applies one-shot with the default interface origin', () => {
    const api = fakeAPI();
    const gesture = new EditGesture(api as any, {
      mode: 'command',
      label: 'Toggle',
      command: { type: 'set_layer', target: 'L1', patch: {} } as any
    });
    gesture.once(true);
    expect(api.edit.apply).toHaveBeenCalledWith(
      expect.objectContaining({ value: true }),
      { label: 'Toggle', origin: 'interface' }
    );
  });

  it('local mode never touches edit or history', () => {
    const api = fakeAPI();
    const set = vi.fn();
    const gesture = new EditGesture(api as any, { mode: 'local', label: 'Tool', set });
    gesture.begin();
    gesture.write(1);
    gesture.commit();
    gesture.once(2);
    expect(set).toHaveBeenCalledTimes(2);
    expect(api.edit.begin).not.toHaveBeenCalled();
    expect(api.history.begin).not.toHaveBeenCalled();
  });

  it('legacy set mode wraps in history', () => {
    const api = fakeAPI();
    const set = vi.fn();
    const gesture = new EditGesture(api as any, { mode: 'set', label: 'Raw', set });
    gesture.begin();
    gesture.write(3);
    gesture.commit();
    expect(api.history.begin).toHaveBeenCalledWith('Raw');
    expect(api.history.commit).toHaveBeenCalledWith('Raw');
    gesture.once(4);
    expect(api.history.do).toHaveBeenCalled();
    expect(set).toHaveBeenLastCalledWith(4);
  });
});
