import { describe, expect, it, vi } from 'vitest';
import { makePM } from './make-pm';

describe('animation frame cache', () => {
  it('evaluates transform and opacity at each frame, including backwards seeks and parent chains', () => {
    const PM = makePM('core/easing', 'core/model', 'core/anim');
    PM.proj = PM.mkProject({ dur: 5 });
    const parent = PM.mkLayer('solid'), child = PM.mkLayer('solid');
    PM.proj.layers = [parent, child]; child.parent = parent.id;
    parent.p['position.x'].v = 0; child.p['position.x'].v = 10;
    PM.animate(parent, 'position.x', [{ t: 0, v: 0 }, { t: 2, v: 200 }], { ease: 'linear' });
    PM.animate(parent, 'opacity', [{ t: 0, v: 100 }, { t: 2, v: 0 }], { ease: 'linear' });
    for (const t of [0, 1, 2, .5, 0]) {
      PM.beginEval(t);
      expect(PM.worldMatrix(child, t)[4]).toBeCloseTo(10 + 100 * t);
      expect(PM.worldOpacity(child, t)).toBeCloseTo(1);
    }
  });

  it('reuses local transforms within a frame and invalidates them after edits', () => {
    const PM = makePM('core/easing', 'core/model', 'core/anim');
    PM.proj = PM.mkProject({ dur: 5 });
    const layer = PM.mkLayer('solid');
    PM.proj.layers = [layer];
    PM.animate(layer, 'position.x', [{ t: 0, v: 0 }, { t: 2, v: 200 }], { ease: 'linear' });
    PM.beginEval(1);
    const before = PM.localMatrix(layer, 1);
    expect(PM.localMatrix(layer, 1)).toBe(before);
    expect(before[4]).toBeCloseTo(100);
    layer.p['position.x'].kf[1].v = 400;
    PM.touch(); PM.beginEval(1);
    expect(PM.localMatrix(layer, 1)[4]).toBeCloseTo(200);
    PM.beginEval(.5);
    expect(PM.localMatrix(layer, .5)[4]).toBeCloseTo(100);
  });

  it('reuses static transforms across frames and detects direct value and channel replacements', () => {
    const PM = makePM('core/easing', 'core/model', 'core/anim');
    PM.proj = PM.mkProject({ dur: 5 });
    const layer = PM.mkLayer('solid'); PM.proj.layers = [layer];
    layer.p['position.x'].v = 10;
    PM.beginEval(0); const first = PM.localMatrix(layer, 0);
    PM.beginEval(1);
    expect(PM.localMatrix(layer, 1)).toBe(first);
    expect(PM.worldMatrix(layer, 1)).toBe(first);
    layer.p['position.x'].v = 20;
    PM.beginEval(2);
    expect(PM.localMatrix(layer, 2)[4]).toBe(20);
    layer.p['position.x'] = PM.P(30);
    PM.beginEval(3);
    expect(PM.localMatrix(layer, 3)[4]).toBe(30);
    layer.p['position.x'].v = 40;
    PM.touch(); PM.beginEval(3);
    expect(PM.localMatrix(layer, 3)[4]).toBe(40);
  });

  it('switches from static transforms to expressions and animation without freezing later samples', () => {
    const PM = makePM('core/easing', 'core/model', 'core/anim');
    PM.proj = PM.mkProject({ dur: 5 });
    const layer = PM.mkLayer('solid'); PM.proj.layers = [layer];
    PM.beginEval(0); PM.localMatrix(layer, 0);
    layer.p['position.x'].expr = 'T * 100';
    for (const time of [1, 2, .5]) {
      PM.beginEval(time); expect(PM.localMatrix(layer, time)[4]).toBe(time * 100);
    }
    layer.p['position.x'].expr = null;
    layer.p['position.x'].v = 25;
    PM.beginEval(3); const stationary = PM.localMatrix(layer, 3);
    expect(stationary[4]).toBe(25);
    PM.beginEval(4); expect(PM.localMatrix(layer, 4)).toBe(stationary);
    PM.animate(layer, 'position.x', [{ t: 0, v: 0 }, { t: 2, v: 200 }]);
    PM.beginEval(1); expect(PM.localMatrix(layer, 1)[4]).toBeCloseTo(100);
    PM.beginEval(.5); expect(PM.localMatrix(layer, .5)[4]).toBeCloseTo(50);
  });

  it('keeps a static child attached to its animated parent across frames', () => {
    const PM = makePM('core/easing', 'core/model', 'core/anim');
    PM.proj = PM.mkProject({ dur: 5 });
    const parent = PM.mkLayer('solid'), child = PM.mkLayer('solid');
    child.parent = parent.id; child.p['position.x'].v = 10;
    PM.proj.layers = [parent, child];
    PM.animate(parent, 'position.x', [{ t: 0, v: 0 }, { t: 2, v: 200 }]);
    for (const time of [0, 1, 2, .5]) {
      PM.beginEval(time); expect(PM.worldMatrix(child, time)[4]).toBeCloseTo(10 + time * 100);
    }
  });

  it('assembles a shared nested group space once per frame, preserving edits and backwards seeks', () => {
    const PM = makePM('core/easing', 'core/model', 'core/anim');
    PM.proj = PM.mkProject({ dur: 5 });
    const outer = PM.mkLayer('group', { p: { 'position.x': 0, 'position.y': 0, rotation: 17 } });
    const inner = PM.mkLayer('group', { p: { 'position.x': 25, 'position.y': 40, 'scale.x': -80 } });
    inner.group = outer.id;
    const children = Array.from({ length: 200 }, (_, index) => {
      const child = PM.mkLayer('solid', { p: { 'position.x': index, 'position.y': 15 } });
      child.group = inner.id;
      return child;
    });
    PM.proj.layers = [outer, inner, ...children];
    PM.animate(outer, 'position.x', [{ t: 0, v: 0 }, { t: 2, v: 200 }]);
    const localMatrix = vi.spyOn(PM, 'localMatrix');
    for (const time of [0, 1, 2, .5]) {
      PM.beginEval(time);
      const expected = children.map(child => PM.mul(PM.transformParentMatrix(child, time), PM.localMatrix(child, time)));
      localMatrix.mockClear();
      children.forEach((child, index) => expect(PM.worldMatrix(child, time)).toEqual(expected[index]));
      expect(localMatrix.mock.calls.filter(([layer]) => (layer as { type: string }).type === 'group')).toHaveLength(2);
    }
    outer.p['position.x'].kf[1].v = 400;
    PM.touch(); PM.beginEval(.5);
    const expected = PM.mul(PM.transformParentMatrix(children[0], .5), PM.localMatrix(children[0], .5));
    expect(PM.worldMatrix(children[0], .5)).toEqual(expected);
  });

  it('preserves parent rigs and invalid group links on the general group-space path', () => {
    const PM = makePM('core/easing', 'core/model', 'core/anim');
    PM.proj = PM.mkProject({ dur: 5 });
    const rig = PM.mkLayer('null', { p: { 'position.x': 25, rotation: 29 } });
    const outer = PM.mkLayer('group', { p: { 'scale.y': 70, skew: 13 } });
    const inner = PM.mkLayer('group', { p: { 'position.y': 30 } });
    const child = PM.mkLayer('solid');
    inner.group = outer.id; child.group = inner.id;
    PM.proj.layers = [rig, outer, inner, child];
    for (const setup of [
      () => { outer.parent = rig.id; },
      () => { outer.parent = null; child.parent = rig.id; },
      () => { child.parent = null; outer.group = inner.id; },
      () => { outer.group = 'missing'; },
    ]) {
      setup(); PM.touch(); PM.beginEval(1);
      const expected = PM.mul(PM.transformParentMatrix(child, 1), PM.localMatrix(child, 1));
      expect(PM.worldMatrix(child, 1)).toEqual(expected);
    }
  });
});
