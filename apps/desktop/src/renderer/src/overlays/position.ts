export type CursorMenuPlacement = {
  left: number;
  top: number;
  side: 'above' | 'below';
};

/** Place a context menu wholly to one side of its pointer whenever the
 * viewport allows it. If an unusually tall menu fits on neither side, use the
 * roomier side and clamp only as a last resort. */
export function positionMenuAtCursor(
  x: number,
  y: number,
  width: number,
  height: number,
  viewportWidth: number,
  viewportHeight: number,
  gap = 6,
  inset = 6,
): CursorMenuPlacement {
  const clamp = (value: number, minimum: number, maximum: number) =>
    Math.min(Math.max(value, minimum), Math.max(minimum, maximum));
  const left = clamp(x, inset, viewportWidth - width - inset);
  const belowTop = y + gap;
  const aboveTop = y - gap - height;
  const fitsBelow = belowTop >= inset && belowTop + height <= viewportHeight - inset;
  const fitsAbove = aboveTop >= inset && aboveTop + height <= viewportHeight - inset;
  if (fitsBelow) return { left, top: belowTop, side: 'below' };
  if (fitsAbove) return { left, top: aboveTop, side: 'above' };

  const belowSpace = viewportHeight - inset - belowTop;
  const aboveSpace = y - gap - inset;
  const side = aboveSpace > belowSpace ? 'above' : 'below';
  const preferredTop = side === 'above' ? aboveTop : belowTop;
  return {
    left,
    top: clamp(preferredTop, inset, viewportHeight - height - inset),
    side,
  };
}
