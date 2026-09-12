import { describe, expect, it, vi } from 'vitest';
import { inspectorPM } from './multi-edit';

function fixture() {
  const keys = [{ i: 'a', t: 0, v: 10 }, { i: 'b', t: 2, v: 30 }, { i: 'c', t: 4, v: 90 }];
  const layer = { id: 'layer', from: 5, p: { x: { v: 10, kf: keys } } };
  const dispatch = vi.fn((commands: any) => {
    for (const command of [].concat(commands) as any[]) {
      const key = keys.find(key => key.t === command.time - layer.from);
      if (key) key.v = command.value;
    }
  });
  const PM = { sel: { keys: ['a', 'b'] }, time: 6, selLayers: () => [layer], L: () => layer,
    evP: () => (keys[0]!.v + keys[1]!.v) / 2,
    Edit: { begin: vi.fn(), dispatch, apply: dispatch, commit: vi.fn(), cancel: vi.fn() } };
  return { api: inspectorPM(PM), keys, PM, dispatch,
    command: (value: number) => ({ type: 'set_property', target: 'layer', path: 'x', value, time: 6, mode: 'auto' }) };
}

describe('selected keyframe numeric editing', () => {
  it('offsets selected keys equally when typing between keys without changing other keys', () => {
    const f = fixture();
    f.api.Edit.apply(f.command(25));
    expect(f.keys.map(key => key.v)).toEqual([15, 35, 90]);
    expect(f.dispatch.mock.calls[0]![0].map((c: any) => c.time)).toEqual([5, 7]);
  });
  it('uses the gesture starting values across repeated scrubs and resets after commit', () => {
    const f = fixture();
    f.api.Edit.begin();
    f.api.Edit.dispatch(f.command(25));
    f.api.Edit.dispatch(f.command(35));
    expect(f.keys.map(key => key.v)).toEqual([25, 45, 90]);
    f.api.Edit.commit();
    f.api.Edit.apply(f.command(40));
    expect(f.keys.map(key => key.v)).toEqual([30, 50, 90]);
  });
  it('keeps ordinary playhead edits when multiple keys are not selected', () => {
    const f = fixture();
    f.PM.sel.keys = ['a'];
    f.api.Edit.apply(f.command(25));
    expect(f.dispatch).toHaveBeenCalledWith(f.command(25));
  });
});
