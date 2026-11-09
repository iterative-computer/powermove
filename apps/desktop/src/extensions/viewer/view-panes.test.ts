import { describe, expect, it } from 'vitest';
import { fitFrame, paneRects } from './view-panes';

describe('view pane geometry', () => {
  it('splits the viewer into 1, 2 or 4 panes with a hairline between them', () => {
    expect(paneRects(1, 800, 600)).toEqual([{ x: 0, y: 0, w: 800, h: 600 }]);
    expect(paneRects(2, 801, 600)).toEqual([{ x: 0, y: 0, w: 400, h: 600 }, { x: 401, y: 0, w: 400, h: 600 }]);
    const four = paneRects(4, 801, 601);
    expect(four.map(r => [r.x, r.y])).toEqual([[0, 0], [401, 0], [0, 301], [401, 301]]);
    expect(four.every(r => r.w === 400 && r.h === 300)).toBe(true);
  });
  it('fits the composition frame in a pane like the viewer does', () => {
    const frame = fitFrame({ w: 416, h: 300 }, { w: 1920, h: 1080 });
    expect(frame.w / frame.h).toBeCloseTo(1920 / 1080, 5);
    expect(frame.x).toBeCloseTo(8, 5);
    expect(frame.y).toBeGreaterThan(8);
  });
});
