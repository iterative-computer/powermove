import { describe, expect, it, vi } from 'vitest';
import type { EditCommand, PowermoveAPI } from 'powermove';
import { createInspectorEdit } from './multi-edit';

function makeFixture() {
  const keys = [{ i: 'a', t: 0, v: 10 }, { i: 'b', t: 2, v: 30 }, { i: 'c', t: 4, v: 90 }];
  const layer = { id: 'layer', type: 'solid', name: 'Layer', from: 5, dur: 10, lock: false, p: { x: { v: 10, kf: keys } }, d: {}, fx: [], masks: [] };
  const dispatch = vi.fn((input: EditCommand | EditCommand[]) => {
    for (const command of Array.isArray(input) ? input : [input]) {
      if (command.type !== 'set_property') continue;
      const key = keys.find((candidate) => candidate.t === (command.time ?? 0) - layer.from);
      if (key) key.v = Number(command.value);
    }
    return { ok: true };
  });
  const [first, second] = keys;
  if (!first || !second) throw new Error('fixture requires two selected keyframes');
  const selection = { layers: ['layer'], keys: ['a', 'b'], chan: null };
  const api = {
    model: { layer: () => layer },
    selection: { layers: () => selection.layers, keys: () => selection.keys },
    groups: { ancestors: () => [] },
    transport: { time: () => 6 },
    anim: {
      findProp: () => layer.p.x,
      evP: () => (first.v + second.v) / 2
    },
    edit: { begin: vi.fn(), dispatch, apply: dispatch, commit: vi.fn(), cancel: vi.fn(), mutate: vi.fn() }
  } as unknown as PowermoveAPI;
  return {
    edit: createInspectorEdit(api),
    keys,
    selection,
    dispatch,
    command: (value: number) => ({ type: 'set_property', target: 'layer', path: 'x', value, time: 6, mode: 'auto' } as EditCommand)
  };
}

describe('selected keyframe numeric editing', () => {
  it('offsets selected keys equally when typing between keys without changing other keys', () => {
    const fixture = makeFixture();
    fixture.edit.apply(fixture.command(25));
    expect(fixture.keys.map((key) => key.v)).toEqual([15, 35, 90]);
    const dispatched = fixture.dispatch.mock.calls[0]?.[0];
    expect(Array.isArray(dispatched)
      ? dispatched.map((command) => command.type === 'set_property' ? command.time : undefined)
      : []).toEqual([5, 7]);
  });

  it('uses the gesture starting values across repeated scrubs and resets after commit', () => {
    const fixture = makeFixture();
    fixture.edit.begin('Edit');
    fixture.edit.dispatch(fixture.command(25));
    fixture.edit.dispatch(fixture.command(35));
    expect(fixture.keys.map((key) => key.v)).toEqual([25, 45, 90]);
    fixture.edit.commit();
    fixture.edit.apply(fixture.command(40));
    expect(fixture.keys.map((key) => key.v)).toEqual([30, 50, 90]);
  });

  it('keeps ordinary playhead edits when multiple keys are not selected', () => {
    const fixture = makeFixture();
    fixture.selection.keys = ['a'];
    fixture.edit.apply(fixture.command(25));
    expect(fixture.dispatch).toHaveBeenCalledWith(fixture.command(25));
  });
});
