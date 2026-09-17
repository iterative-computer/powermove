import { describe, expect, it } from 'vitest';

import { alignDelta, alignFrame, deltaInParent, unionBox } from './align';

const box = { x0: 100, y0: 50, x1: 300, y1: 150 };
const frame = { x0: 0, y0: 0, x1: 1920, y1: 1080 };

describe('alignment math', () => {
  it('moves a box so the chosen edge or center meets the frame', () => {
    expect(alignDelta(box, frame, 'left')).toEqual({ dx: -100, dy: 0 });
    expect(alignDelta(box, frame, 'right')).toEqual({ dx: 1620, dy: 0 });
    expect(alignDelta(box, frame, 'hcenter')).toEqual({ dx: 760, dy: 0 });
    expect(alignDelta(box, frame, 'top')).toEqual({ dx: 0, dy: -50 });
    expect(alignDelta(box, frame, 'bottom')).toEqual({ dx: 0, dy: 930 });
    expect(alignDelta(box, frame, 'vcenter')).toEqual({ dx: 0, dy: 440 });
  });

  it('aligns one layer to the composition and several to their shared bounds', () => {
    expect(alignFrame([box], { w: 800, h: 450 })).toEqual({ x0: 0, y0: 0, x1: 800, y1: 450 });
    const other = { x0: 400, y0: 20, x1: 500, y1: 400 };
    expect(alignFrame([box, other], { w: 800, h: 450 })).toEqual({ x0: 100, y0: 20, x1: 500, y1: 400 });
    expect(unionBox([])).toBeNull();
  });

  it('expresses a world delta in a scaled or rotated parent space', () => {
    expect(deltaInParent(null, 10, 20)).toEqual({ dx: 10, dy: 20 });
    expect(deltaInParent([2, 0, 0, 2, 500, 500], 10, 20)).toEqual({ dx: 5, dy: 10 });
    const rotated = deltaInParent([0, 1, -1, 0, 0, 0], 10, 0); // parent rotated 90°
    expect(rotated.dx).toBeCloseTo(0, 9);
    expect(rotated.dy).toBeCloseTo(-10, 9);
    expect(deltaInParent([0, 0, 0, 0, 0, 0], 3, 4)).toEqual({ dx: 3, dy: 4 });
  });
});
