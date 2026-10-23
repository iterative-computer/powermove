import { describe, expect, it } from 'vitest';
import { makePM } from '../__tests__/make-pm';
import { compositingLayers } from '../gl/compositor';
import { sourceTime } from './retiming';

function editor() {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history');
  PM.proj = PM.mkProject({ w: 640, h: 360, fps: 30, dur: 3 });
  const group = PM.mkLayer('group', { name: 'Shared opacity', p: { opacity: 50 } });
  const first = PM.mkLayer('shape', { from: .3, dur: 2.7, d: { w: 100, h: 100 } });
  const second = PM.mkLayer('shape', { from: .3, dur: 2.7, d: { w: 100, h: 100 } });
  first.group = second.group = group.id;
  PM.proj.layers = [group, first, second]; PM.ProjectIndex.invalidate(); PM.touch();
  return { PM, group, first, second };
}

// Both opaque shapes cover the same pixel. Use the real grouping plan and
// precomp timing, then source-over alpha at each isolated boundary without a GPU.
function overlappingAlpha(PM: any, comp: any, time: number, parent: string | null = null): number {
  PM.scope.push(comp);
  try {
    const layers = compositingLayers(comp.layers, parent, layer => PM.ev(layer, 'opacity', time) < 100);
    let accumulated = 0;
    for (const layer of layers.slice().reverse()) {
      if (!PM.active(layer, time) || (layer.type !== 'group' && PM.TYPE_META[layer.type]?.visual === false)) continue;
      const own = PM.ev(layer, 'opacity', time) / 100;
      const source = layer.type === 'group' ? overlappingAlpha(PM, comp, time, layer.id)
        : layer.type === 'precomp' ? overlappingAlpha(PM, PM.proj.comps[layer.d.comp], sourceTime(PM, layer, time)) : 1;
      const alpha = own * source;
      accumulated = alpha + accumulated * (1 - alpha);
    }
    return accumulated;
  } finally { PM.scope.pop(); }
}

describe('precompose shared group opacity', () => {
  it.each([false, true])('preserves the shared opacity boundary when only one child moves (adjustDuration: %s)', adjustDuration => {
    const { PM, first } = editor();
    expect(overlappingAlpha(PM, PM.proj, 1)).toBe(.5);
    PM.Comps.precompose([first.id], { adjustDuration });
    expect(overlappingAlpha(PM, PM.proj, 1)).toBe(.5);
  });

  it('preserves animated opacity at every frame of a shared group', () => {
    const { PM, group, first } = editor();
    group.p.opacity.kf = [PM.KF(0, 20), PM.KF(3, 80)];
    PM.touch();
    const times = [.5, 1.5, 2.5];
    const before = times.map(time => overlappingAlpha(PM, PM.proj, time));
    PM.Comps.precompose([first.id], { adjustDuration: true });
    times.forEach((time, index) => expect(overlappingAlpha(PM, PM.proj, time)).toBeCloseTo(before[index]!, 6));
  });

  it('keeps full group precomposition unchanged', () => {
    const { PM, group } = editor();
    const before = overlappingAlpha(PM, PM.proj, 1);
    PM.Comps.precompose([group.id]);
    expect(overlappingAlpha(PM, PM.proj, 1)).toBe(before);
  });

  it('keeps identity groups with an offset anchor and nested shared styles outside', () => {
    const { PM, group, first } = editor();
    const outer = PM.mkLayer('group', { p: { opacity: 80 } });
    group.group = outer.id;
    group.p['anchor.x'].v = group.p['position.x'].v = 150;
    group.p['anchor.y'].v = group.p['position.y'].v = 90;
    group.blend = 'multiply'; group.fx = [{ id: 'shared-blur', type: 'blur' }];
    group.masks = [{ id: 'shared-mask', on: PM.P(true) }];
    PM.proj.layers.unshift(outer); PM.ProjectIndex.invalidate(); PM.touch();
    expect(overlappingAlpha(PM, PM.proj, 1)).toBeCloseTo(.4);
    const id = PM.Comps.precompose([first.id]);
    const wrapper = PM.proj.layers.find((layer: any) => layer.type === 'precomp');
    expect(wrapper.group).toBe(group.id);
    expect(PM.proj.layers.find((layer: any) => layer.id === group.id)).toBe(group);
    expect(group.fx).toHaveLength(1); expect(group.masks).toHaveLength(1);
    const rigs = PM.proj.comps[id].layers.filter((layer: any) => layer.type === 'group');
    expect(rigs).toHaveLength(2);
    rigs.forEach((rig: any) => {
      expect(rig.p.opacity.v).toBe(100); expect(rig.fx).toEqual([]);
      expect(rig.masks).toEqual([]); expect(rig.blend).toBe('normal');
    });
    expect(overlappingAlpha(PM, PM.proj, 1)).toBeCloseTo(.4);
  });

  it('preserves one shared boundary and links through Undo and redo', () => {
    const { PM, group, first } = editor();
    const id = PM.Comps.precompose([first.id], { adjustDuration: true });
    expect(PM.hist.undo()).toBe(true);
    expect(PM.proj.comps[id]).toBeUndefined();
    expect(PM.proj.layers.find((layer: any) => layer.id === first.id).group).toBe(group.id);
    expect(overlappingAlpha(PM, PM.proj, 1)).toBe(.5);
    expect(PM.hist.redo()).toBe(true);
    expect(PM.proj.layers.find((layer: any) => layer.type === 'precomp').group).toBe(group.id);
    expect(overlappingAlpha(PM, PM.proj, 1)).toBe(.5);
  });

  it.each(['offcanvas transform', 'animated transform', 'expression transform', 'parented group', '3D group', '3D child', 'mixed membership', 'moved matte'])(
    'rejects a partial styled boundary with %s before mutation or history', kind => {
      const { PM, group, first } = editor();
      const ids = [first.id];
      if (kind === 'offcanvas transform') { group.p['position.x'].v = -900; first.p['position.x'].v = 1000; }
      if (kind === 'animated transform') group.p.rotation.kf = [PM.KF(0, 0), PM.KF(3, 90)];
      if (kind === 'expression transform') group.p.rotation.expr = 'time * 30';
      if (kind === 'parented group') {
        const parent = PM.mkLayer('null'); group.parent = parent.id; PM.proj.layers.push(parent);
      }
      if (kind === '3D group') group.threeD = true;
      if (kind === '3D child') first.threeD = true;
      if (kind === 'mixed membership') {
        const outside = PM.mkLayer('shape'); PM.proj.layers.push(outside); ids.push(outside.id);
      }
      if (kind === 'moved matte') group.matteSource = first.id;
      PM.ProjectIndex.invalidate(); PM.touch();
      const before = JSON.stringify(PM.proj), history = PM.hist.list().length;
      expect(() => PM.Comps.precompose(ids)).toThrow('Select the whole styled group');
      expect(JSON.stringify(PM.proj)).toBe(before);
      expect(PM.hist.list()).toHaveLength(history);
      expect(PM.hist.undo()).toBe(false);
    });

  it('still allows a whole styled group with an animated transform', () => {
    const { PM, group } = editor();
    group.p.rotation.kf = [PM.KF(0, 0), PM.KF(3, 90)]; PM.touch();
    const id = PM.Comps.precompose([group.id]);
    expect(PM.proj.comps[id].layers.find((layer: any) => layer.id === group.id)).toBe(group);
    expect(overlappingAlpha(PM, PM.proj, 1)).toBe(.5);
  });

});
