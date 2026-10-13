// @vitest-environment happy-dom
import { describe, expect, it } from 'vitest';

import { rawDragMoves, supportsRawUpdates } from './raw-gestures';

function rawUpdate(type: string, clientX: number, clientY: number, pointerId = 1): Event {
  const event = new Event(type, { bubbles: true });
  Object.assign(event, { clientX, clientY, pointerId, shiftKey: false });
  return event;
}

describe('engine-path gestures read pointerrawupdate', () => {
  it('drives moves from raw updates with the drag helper\'s deltas and ignores its pointermove moves', () => {
    const target = new EventTarget();
    const moves: [number, number][] = [];
    const drag = rawDragMoves({ clientX: 100, clientY: 50, pointerId: 1 }, (dx, dy) => moves.push([dx, dy]), target);
    target.dispatchEvent(rawUpdate('pointerrawupdate', 110, 55));
    target.dispatchEvent(rawUpdate('pointerrawupdate', 130, 40));
    // The helper's own rAF-aligned move is a no-op.
    drag.move(30, -10, rawUpdate('pointermove', 130, 40) as PointerEvent);
    expect(moves).toEqual([[10, 5], [30, -10]]);
    expect(drag.updates()).toBe(2);
  });

  it('ignores other pointers and stops on up or cancel', () => {
    const target = new EventTarget();
    const moves: number[] = [];
    const drag = rawDragMoves({ clientX: 0, clientY: 0, pointerId: 7 }, (dx) => moves.push(dx), target);
    target.dispatchEvent(rawUpdate('pointerrawupdate', 5, 0, 8));
    target.dispatchEvent(rawUpdate('pointerrawupdate', 6, 0, 7));
    drag.stop();
    drag.stop();
    target.dispatchEvent(rawUpdate('pointerrawupdate', 9, 0, 7));
    expect(moves).toEqual([6]);
  });

  it('knows whether a window dispatches raw updates', () => {
    expect(supportsRawUpdates(null)).toBe(false);
    expect(supportsRawUpdates({ onpointerrawupdate: null } as unknown as Window)).toBe(true);
    expect(supportsRawUpdates({} as unknown as Window)).toBe(false);
  });
});
