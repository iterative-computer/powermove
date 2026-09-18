import { describe, expect, it } from 'vitest';

import { positionMenuAtCursor } from './position';

describe('context menu cursor placement', () => {
  it('opens below a high cursor when the menu fits', () => {
    expect(positionMenuAtCursor(80, 24, 180, 400, 800, 600)).toEqual({
      left: 80,
      top: 30,
      side: 'below',
    });
  });

  it('opens entirely above a low cursor when there is room', () => {
    expect(positionMenuAtCursor(80, 520, 180, 400, 800, 600)).toEqual({
      left: 80,
      top: 114,
      side: 'above',
    });
  });

  it('keeps the menu inside the horizontal screen inset', () => {
    expect(positionMenuAtCursor(790, 100, 180, 120, 800, 600).left).toBe(614);
  });

  it('uses the roomier side for a menu too tall to fit either side', () => {
    expect(positionMenuAtCursor(80, 460, 180, 540, 800, 600)).toMatchObject({
      top: 6,
      side: 'above',
    });
  });
});

it.each([-100, 900])('keeps menus visible after a cursor falls outside the viewport: %s', y => {
  const result = positionMenuAtCursor(80, y, 180, 120, 800, 600);
  expect(result.top).toBeGreaterThanOrEqual(6);
  expect(result.top + 120).toBeLessThanOrEqual(594);
});
