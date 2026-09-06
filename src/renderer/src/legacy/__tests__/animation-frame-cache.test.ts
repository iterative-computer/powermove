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
});
