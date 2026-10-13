import { describe, expect, it } from 'vitest';
import { makePM } from '../__tests__/make-pm';
import { compositingLayers } from '../gl/compositor';
import { temporalKeys } from './temporal-bridge';

function editor(count = 2) {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
  PM.proj = PM.mkProject({ w: 640, h: 360, fps: 10, dur: 3 }); PM.time = 1.5;
  const children = Array.from({ length: count }, () => PM.mkLayer('shape', { p: { 'position.x': 200, 'position.y': 180 } }));
  PM.proj.layers = [...children]; PM.ProjectIndex.invalidate(); PM.touch();
  const group = PM.groupLayers(children.map(layer => layer.id)); PM.hist.clear();
  return { PM, group, children };
}

// Coincident opaque shapes cover this pixel. Use the real grouping plan and
// eligibility, then apply source-over alpha at each isolated boundary.
function alpha(PM: any, time: number, parent: string | null = null): number {
  const layers = compositingLayers(PM.proj.layers, parent, layer => PM.ev(layer, 'opacity', time) < 100);
  let accumulated = 0;
  for (const layer of layers.slice().reverse()) {
    if (!PM.active(layer, time) || (layer.type !== 'group' && PM.TYPE_META[layer.type]?.visual === false)) continue;
    const source = layer.type === 'group' ? alpha(PM, time, layer.id) : 1;
    const own = PM.clamp(PM.ev(layer, 'opacity', time) / 100, 0, 1) * source;
    accumulated = own + accumulated * (1 - own);
  }
  return accumulated;
}
function state(PM: any) { return JSON.stringify({ source: PM.serialize(), selection: PM.sel, history: PM.hist.list() }); }
function apply(PM: any, ids: string[]) { return PM.Edit.apply({ type: 'ungroup_layers', targets: ids }, { origin: 'agent', label: 'Ungroup layers' }); }
function rejected(PM: any, ids: string[]) {
  const before = state(PM), result = apply(PM, ids);
  expect(result.ok).toBe(false); expect(result.message).toContain('Pre-compose this group');
  expect(state(PM)).toBe(before); expect(PM.hist.undo()).toBe(false);
}
const times = [.5, 1.5, 2.5];

describe('ungroup shared opacity safety', () => {
  it.each(['static', 'fade', 'time expression'])('refuses overlapping children with %s opacity without changing their image', mode => {
    const { PM, group } = editor(); group.p.opacity.v = 50;
    if (mode === 'fade') group.p.opacity.kf = [PM.KF(0, 20), PM.KF(3, 80)];
    if (mode === 'time expression') group.p.opacity.expr = '20 + time * 20';
    PM.touch(); const before = times.map(time => alpha(PM, time));
    expect(before[1]).toBeCloseTo(.5); rejected(PM, [group.id]);
    times.forEach((time, index) => expect(alpha(PM, time)).toBeCloseTo(before[index]!));
  });

  it.each(['static', 'fade', 'time expression'])('keeps a plain single child editable with %s opacity and Undo', mode => {
    const { PM, group, children: [child] } = editor(1); group.p.opacity.v = 50; child!.p.opacity.v = 80;
    if (mode === 'fade') group.p.opacity.kf = [PM.KF(0, 20), PM.KF(3, 80)];
    if (mode === 'time expression') group.p.opacity.expr = '20 + time * 20';
    PM.touch(); const before = times.map(time => alpha(PM, time));
    expect(apply(PM, [group.id]).ok).toBe(true); expect(PM.L(group.id)).toBeNull();
    times.forEach((time, index) => expect(alpha(PM, time)).toBeCloseTo(before[index]!, 6));
    expect(PM.hist.list()).toEqual(['Ungroup layers']); expect(PM.hist.undo()).toBe(true);
    expect(PM.L(group.id).type).toBe('group'); expect(PM.hist.redo()).toBe(true);
    times.forEach((time, index) => expect(alpha(PM, time)).toBeCloseTo(before[index]!, 6));
  });

  it.each([false, true])('keeps ordinary 100% multi-child groups supported (constant keys: %s)', keys => {
    const { PM, group } = editor();
    if (keys) { group.p.opacity.v = 75; group.p.opacity.kf = [PM.KF(0, 100, 'linear'), PM.KF(3, 100, 'linear')]; }
    PM.touch(); expect(apply(PM, [group.id]).ok).toBe(true);
    times.forEach(time => expect(alpha(PM, time)).toBe(1));
  });

  it('recognizes opacity dips between equal 100% endpoints with nonzero velocity', () => {
    const { PM, group } = editor(); group.p.opacity.kf = [PM.KF(0, 100), PM.KF(3, 100)]; temporalKeys(group.p.opacity.kf);
    group.p.opacity.kf[0].outInterp = 'bezier'; group.p.opacity.kf[0].outEase = { speed: -80, influence: 33 };
    group.p.opacity.kf[1].inInterp = 'bezier'; group.p.opacity.kf[1].inEase = { speed: 80, influence: 33 }; PM.touch();
    expect(PM.ev(group, 'opacity', 1.5)).toBeLessThan(100); rejected(PM, [group.id]);
  });

  it.each([false, true])('preserves opacity-only nested groups with one drawable leaf (both selected: %s)', both => {
    const { PM, group, children: [child] } = editor(1), outer = PM.groupLayers([group.id]);
    outer.p.opacity.v = 80; group.p.opacity.v = 50; child!.p.opacity.v = 80; PM.touch(); PM.hist.clear();
    expect(alpha(PM, 1.5)).toBeCloseTo(.32);
    expect(apply(PM, both ? [outer.id, group.id] : [outer.id]).ok).toBe(true);
    times.forEach(time => expect(alpha(PM, time)).toBeCloseTo(.32));
  });

  it('refuses a nested group containing multiple drawable leaves', () => {
    const { PM, group } = editor(), outer = PM.groupLayers([group.id]);
    outer.p.opacity.v = 80; group.p.opacity.v = 50; PM.touch(); PM.hist.clear(); rejected(PM, [outer.id]);
  });

  it.each(['effect', 'mask', 'matte', 'blend', 'transition', 'motion blur', 'transform expression', 'content expression', 'visibility expression', 'custom definition', '3D', 'hidden group', 'solo group'])(
    'refuses an unsafe single child with %s', mode => {
      const { PM, group, children: [child] } = editor(1); group.p.opacity.v = 50;
      const leaf = child!;
      if (mode === 'effect') leaf.fx = [{ id: 'fx', type: 'blur', on: true, p: {} }];
      if (mode === 'mask') leaf.masks = [{ id: 'mask', on: true, p: {} }];
      if (mode === 'matte') leaf.matteSource = 'another-layer';
      if (mode === 'blend') leaf.blend = 'multiply';
      if (mode === 'transition') leaf.transitionIn = { type: 'crossfade', dur: 1, p: {} };
      if (mode === 'motion blur') leaf.mblur = true;
      if (mode === 'transform expression') leaf.p['position.x'].expr = '200 + time * 10';
      if (mode === 'content expression') leaf.d.color = PM.P('#ffffff', { expr: 'layer.p.opacity.v > 80 ? "#ff0000" : "#0000ff"' });
      if (mode === 'visibility expression') leaf.on = PM.P(true, { expr: 'layer.p.opacity.v > 80' });
      if (mode === 'custom definition') { leaf.type = 'extension'; leaf.d.definition = 'custom-fill'; }
      if (mode === '3D') leaf.threeD = true;
      if (mode === 'hidden group') group.on = false;
      if (mode === 'solo group') group.solo = true;
      PM.touch(); rejected(PM, [group.id]);
    });

  it.each([200, -50])('clamps the removed group opacity %s before transferring it to a plain child', value => {
    const { PM, group, children: [child] } = editor(1); group.p.opacity.v = value; child!.p.opacity.v = 80;
    PM.touch(); const before = times.map(time => alpha(PM, time));
    expect(apply(PM, [group.id]).ok).toBe(true);
    times.forEach((time, index) => expect(alpha(PM, time)).toBeCloseTo(before[index]!));
    expect(PM.hist.undo()).toBe(true); expect(PM.hist.redo()).toBe(true);
    times.forEach((time, index) => expect(alpha(PM, time)).toBeCloseTo(before[index]!));
  });

  it('clamps an animated group expression at every baked frame', () => {
    const { PM, group, children: [child] } = editor(1); group.p.opacity.expr = 'time * 100 - 100'; child!.p.opacity.v = 80;
    PM.touch(); const before = times.map(time => alpha(PM, time)); expect(before).toEqual([0, .4, .8]);
    expect(apply(PM, [group.id]).ok).toBe(true);
    times.forEach((time, index) => expect(alpha(PM, time)).toBeCloseTo(before[index]!));
    expect(PM.hist.undo()).toBe(true); expect(PM.hist.redo()).toBe(true);
    times.forEach((time, index) => expect(alpha(PM, time)).toBeCloseTo(before[index]!));
  });

  it.each(['overrange value', 'negative value', 'curve overshoot', 'spring'])('refuses an attenuated single child with %s opacity', mode => {
    const { PM, group, children: [child] } = editor(1); group.p.opacity.v = 50;
    if (mode === 'overrange value') child!.p.opacity.v = 200;
    if (mode === 'negative value') child!.p.opacity.v = -50;
    if (mode === 'curve overshoot') {
      child!.p.opacity.kf = [PM.KF(0, 100), PM.KF(3, 100)]; temporalKeys(child!.p.opacity.kf);
      child!.p.opacity.kf[0].outInterp = 'bezier'; child!.p.opacity.kf[0].outEase = { speed: 80, influence: 33 };
      child!.p.opacity.kf[1].inInterp = 'bezier'; child!.p.opacity.kf[1].inEase = { speed: -80, influence: 33 };
    }
    if (mode === 'spring') { child!.p.opacity.kf = [PM.KF(0, 0), PM.KF(3, 100)]; child!.p.opacity.kf[0].spring = { stiffness: 100, damping: 2, mass: 1 }; temporalKeys(child!.p.opacity.kf); }
    PM.touch(); rejected(PM, [group.id]);
  });

  it('keeps a bounded child opacity fade supported', () => {
    const { PM, group, children: [child] } = editor(1); group.p.opacity.v = 50;
    child!.p.opacity.kf = [PM.KF(0, 20, 'linear'), PM.KF(3, 80, 'linear')]; PM.touch();
    const before = times.map(time => alpha(PM, time)); expect(apply(PM, [group.id]).ok).toBe(true);
    times.forEach((time, index) => expect(alpha(PM, time)).toBeCloseTo(before[index]!));
    expect(PM.hist.undo()).toBe(true); expect(PM.hist.redo()).toBe(true);
    times.forEach((time, index) => expect(alpha(PM, time)).toBeCloseTo(before[index]!));
  });

  it('rejects legacy child opacity without migrating it during a direct call', () => {
    const { PM, group, children: [child] } = editor(1); group.p.opacity.v = 50;
    child!.p.opacity.kf = [PM.KF(0, 100), PM.KF(3, 0)]; child!.p.opacity.kf[0].eo = [.5, 2]; PM.touch();
    const before = state(PM);
    expect(() => PM.ungroupLayers([group.id])).toThrow('Pre-compose this group'); expect(state(PM)).toBe(before);
    expect(PM.hist.undo()).toBe(false);
  });

  it('does not change an overrange child when removing a 100% group', () => {
    const { PM, group, children: [child] } = editor(1); child!.p.opacity.v = 200; PM.touch();
    expect(apply(PM, [group.id]).ok).toBe(true); expect(PM.L(child!.id).p.opacity.v).toBe(200); expect(alpha(PM, 1.5)).toBe(1);
  });

  it('allows an empty group with opacity animation', () => {
    const { PM } = editor(1);
    const group = PM.mkLayer('group'); group.p.opacity.kf = [PM.KF(0, 20), PM.KF(3, 80)];
    PM.proj.layers = [group]; PM.ProjectIndex.invalidate(); PM.touch(); PM.hist.clear();
    expect(apply(PM, [group.id]).ok).toBe(true); expect(PM.proj.layers).toEqual([]);
    expect(PM.hist.undo()).toBe(true); expect(PM.L(group.id).type).toBe('group');
  });

  it.each(['agent command', 'direct core call'])('validates all selected groups before touching the first safe group (%s)', mode => {
    const { PM, group: safe } = editor(1); safe.p.opacity.v = 50;
    const extra = [PM.mkLayer('shape'), PM.mkLayer('shape')]; PM.proj.layers.push(...extra); PM.ProjectIndex.invalidate(); PM.touch();
    const unsafe = PM.groupLayers(extra.map(layer => layer.id)); unsafe.p.opacity.v = 50;
    PM.selectLayers([safe.id, unsafe.id]); PM.touch(); PM.hist.clear();
    const before = state(PM);
    if (mode === 'agent command') rejected(PM, [safe.id, unsafe.id]);
    else expect(() => PM.ungroupLayers([safe.id, unsafe.id])).toThrow('Pre-compose this group');
    expect(state(PM)).toBe(before);
  });
});
