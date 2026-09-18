import { expect, it } from 'vitest';
import { planGraphKeyframeMove, planGraphKeyframeScale } from './graph-selection';

it('indexes neighbours once for a 10,000-key move and scale', () => {
  let propertyReads = 0;
  const items = Array.from({ length: 10000 }, (_, i) => ({
    id: String(i), get property() { propertyReads++; return i % 10; },
    time: Math.floor(i / 10) / 30, selected: i % 7 !== 0, value: i, minTime: 0, maxTime: 100,
  }));
  const move = planGraphKeyframeMove(items, .5, 30);
  const scale = planGraphKeyframeScale(items, { timeScale: 1.2, valueScale: 1.1, anchorTime: 0, anchorValue: 0 }, 30);
  // Every key remains in order and fixed neighbours prevent the requested
  // expansion. The former pairwise search performed hundreds of millions of reads.
  expect(move.delta).toBe(0);
  expect(scale.timeScale).toBe(1);
  expect(scale.moves).toHaveLength(items.filter(item => item.selected).length);
  expect(propertyReads).toBeLessThan(items.length * 10);
});
