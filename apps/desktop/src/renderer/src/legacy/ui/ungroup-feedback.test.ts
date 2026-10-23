// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { makePM } from '../__tests__/make-pm';

afterEach(() => { document.body.replaceChildren(); vi.restoreAllMocks(); });

function editor() {
  const PM = makePM('core/util', 'core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing', 'ui/shortcuts');
  PM.proj = PM.mkProject({ w: 640, h: 360, dur: 3 }); PM.time = 1;
  const children = [PM.mkLayer('shape'), PM.mkLayer('shape')];
  PM.proj.layers = [...children]; PM.ProjectIndex.invalidate(); PM.touch();
  const group = PM.groupLayers(children.map(layer => layer.id)); PM.hist.clear();
  const toast = vi.spyOn(PM, 'toast');
  const state = () => JSON.stringify({ source: PM.serialize(), selection: PM.sel, history: PM.hist.list() });
  return { PM, group, children, toast, state };
}

describe('Ungroup command feedback', () => {
  it.each(['shared opacity', 'visual effect'])('shows the actual %s failure without changing source or history', kind => {
    const { PM, group, toast, state } = editor();
    if (kind === 'shared opacity') group.p.opacity.v = 50;
    else group.fx = [{ id: 'blur', type: 'blur', on: true, p: {} }];
    PM.touch(); const before = state();
    const result = PM.cmd('ungroupLayers');
    expect(result.ok).toBe(false); expect(toast).toHaveBeenCalledExactlyOnceWith(result.message);
    expect(result.message).toContain(kind === 'shared opacity' ? 'Pre-compose this group' : 'Remove visual compositing');
    expect(state()).toBe(before); expect(PM.hist.undo()).toBe(false);
  });

  it('shows an empty-selection failure rather than silently doing nothing', () => {
    const { PM, toast, state } = editor(); PM.selectLayers([]); PM.hist.clear(); const before = state();
    const result = PM.cmd('ungroupLayers');
    expect(result.ok).toBe(false); expect(toast).toHaveBeenCalledExactlyOnceWith(result.message);
    expect(state()).toBe(before); expect(PM.hist.undo()).toBe(false);
  });

  it('ungroups an ordinary group in one Undo operation without an error toast', () => {
    const { PM, group, children, toast } = editor();
    expect(PM.cmd('ungroupLayers').ok).toBe(true); expect(PM.L(group.id)).toBeNull();
    expect(PM.sel.layers).toEqual(children.map(layer => layer.id)); expect(toast).not.toHaveBeenCalled();
    expect(PM.hist.list()).toEqual(['Ungroup layers']); expect(PM.hist.undo()).toBe(true);
    expect(PM.L(group.id).type).toBe('group'); expect(PM.hist.undo()).toBe(false);
  });
});
