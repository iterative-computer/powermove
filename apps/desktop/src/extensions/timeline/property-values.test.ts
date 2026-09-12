import { expect, it } from 'vitest';
import { draggedPropertyValue } from './property-values';
it('scrubs within limits and supports precise and fast adjustment', () => {
  expect(draggedPropertyValue(50, 80, { step: 1, min: 0, max: 100 })).toBe(100);
  expect(draggedPropertyValue(50, -80, { step: 1, min: 0, max: 100 })).toBe(0);
  expect(draggedPropertyValue(50, 10, { step: .5 }, true)).toBe(50.5);
  expect(draggedPropertyValue(50, 10, { step: .5 }, false, true)).toBe(100);
});
