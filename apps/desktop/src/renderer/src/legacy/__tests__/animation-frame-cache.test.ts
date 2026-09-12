import { describe, expect, it } from 'vitest';
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
});
