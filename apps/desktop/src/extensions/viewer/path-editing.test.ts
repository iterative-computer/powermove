import { expect, it, vi } from 'vitest';
import { makePM } from '../../renderer/src/legacy/__tests__/make-pm';
import { makeVectorPath, makeVertex } from '../../renderer/src/legacy/core/vector-paths';
import { startPathEdit } from './path-editing';

function fixture(lock: 'layer' | 'group' | 'nested group' | 'none') {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
  PM.proj = PM.mkProject({ dur: 3 }); PM.time = 0;
  const path = makeVectorPath(PM);
  path.vertices = [makeVertex(PM, 0, 0), makeVertex(PM, 100, 0), makeVertex(PM, 0, 100)];
  const layer = PM.mkLayer('shape', { d: { paths: [path] } });
  PM.proj.layers = [layer]; PM.ProjectIndex.invalidate();
  const inner = PM.groupLayers([layer.id]);
  const outer = PM.groupLayers([inner.id]);
  if (lock === 'layer') layer.lock = true;
  if (lock === 'group') inner.lock = true;
  if (lock === 'nested group') outer.lock = true;
  PM.selectLayers(layer.id); PM.hist.clear();
  const api: any = {
    selection: { first: () => PM.L(layer.id) },
    model: { P: PM.P, mkMask: PM.mkMask },
    util: { uid: PM.uid }, project: { get: () => PM.proj },
    groups: { ancestors: PM.groupAncestors }, edit: PM.Edit, history: PM.hist,
    anim: { worldMatrix: PM.worldMatrix, mul: PM.mul, evP: PM.evP, touch: PM.touch, setKeyOn: PM.setKeyOn },
    transport: { time: () => PM.time, invalidate: vi.fn() }, services: { get: vi.fn() }
  };
  const state = { activePath: path.id, requestOverlay: vi.fn() };
  let drag: any;
  const beginDrag = vi.fn((_event, callbacks) => { drag = callbacks; });
  const event = (x: number, y: number, extra = {}) => {
    const m = PM.worldMatrix(PM.L(layer.id), 0);
    return { x: m[0] * x + m[2] * y + m[4], y: m[1] * x + m[3] * y + m[5], detail: 1, ...extra };
  };
  const start = (action: string) => startPathEdit(api, state,
    action === 'add' ? event(200, 200) : action === 'close' ? event(0, 0, { detail: 2 })
      : event(100, 0, action === 'delete' ? { metaKey: true } : {}),
    e => e, beginDrag, 1);
  return { PM, layer, path, state, event, start, beginDrag, drag: () => drag };
}

for (const lock of ['layer', 'group', 'nested group'] as const) {
  it.each(['add', 'move', 'close', 'delete'])('protects a ' + lock + ' from pen %s edits', action => {
    const f = fixture(lock);
    const before = JSON.stringify(f.PM.proj);
    f.start(action);
    f.drag()?.move(20, 30, f.event(120, 30));
    f.drag()?.up();
    expect(JSON.stringify(f.PM.proj)).toBe(before);
    expect(f.state.activePath).toBe(f.path.id);
    expect(f.beginDrag).not.toHaveBeenCalled();
    expect(f.PM.hist.busy()).toBe(false);
    expect(f.PM.hist.undo()).toBe(false);
  });
}

it.each(['add', 'move', 'close', 'delete'])('keeps unlocked pen %s edits available with Undo', action => {
  const f = fixture('none');
  const before = JSON.stringify(f.PM.proj);
  f.start(action);
  f.drag()?.move(20, 30, f.event(120, 30));
  f.drag()?.up();
  expect(JSON.stringify(f.PM.proj)).not.toBe(before);
  expect(f.PM.hist.undo()).toBe(true);
  expect(JSON.stringify(f.PM.proj)).toBe(before);
  expect(f.PM.hist.undo()).toBe(false);
});
