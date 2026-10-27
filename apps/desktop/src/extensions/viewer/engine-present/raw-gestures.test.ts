// @vitest-environment happy-dom
import { describe, expect, it } from 'vitest';

import { RAW_MOVE_MIN_INTERVAL_MS, rawDragMoves, supportsRawUpdates } from './raw-gestures';

function rawUpdate(type: string, clientX: number, clientY: number, pointerId = 1): Event {
  const event = new Event(type, { bubbles: true });
  Object.assign(event, { clientX, clientY, pointerId, shiftKey: false });
  return event;
}

/** A manual clock, timers, and task queue. */
function clock() {
  let t = 1000;
  let next = 1;
  const pending = new Map<number, { at: number; fn: () => void }>();
  const queued: Array<() => void> = [];
  return {
    now: () => t,
    task: (fn: () => void) => void queued.push(fn),
    /** Runs the tasks posted so far (and those they post). */
    tasks() {
      while (queued.length) queued.shift()!();
    },
    timers: {
      set: (fn: () => void, ms: number) => {
        const id = next++;
        pending.set(id, { at: t + ms, fn });
        return id;
      },
      clear: (id: number) => void pending.delete(id)
    },
    pending: () => pending.size,
    /** Runs the queued tasks, then moves the clock on, firing due timers and the tasks they post. */
    advance(ms: number) {
      this.tasks();
      t += ms;
      for (const [id, timer] of [...pending]) {
        if (timer.at > t) continue;
        pending.delete(id);
        timer.fn();
      }
      this.tasks();
    }
  };
}

describe('engine-path gestures read pointerrawupdate', () => {
  it('drives moves from raw updates with the drag helper\'s deltas and ignores its pointermove moves', () => {
    const target = new EventTarget();
    const c = clock();
    const moves: [number, number][] = [];
    const drag = rawDragMoves({ clientX: 100, clientY: 50, pointerId: 1 }, (dx, dy) => moves.push([dx, dy]), target, c);
    target.dispatchEvent(rawUpdate('pointerrawupdate', 110, 55));
    c.advance(8);
    target.dispatchEvent(rawUpdate('pointerrawupdate', 130, 40));
    // The helper's own rAF-aligned move is a no-op.
    drag.move(30, -10, rawUpdate('pointermove', 130, 40) as PointerEvent);
    c.tasks();
    expect(moves).toEqual([[10, 5], [30, -10]]);
    expect(drag.updates()).toBe(2);
    expect(drag.applied()).toBe(2);
  });

  it('never moves inside the raw update: the move runs in the next task, and updates waiting for it collapse into the newest', () => {
    const target = new EventTarget();
    const c = clock();
    const moves: number[] = [];
    rawDragMoves({ clientX: 0, clientY: 0 }, (dx) => moves.push(dx), target, c);
    target.dispatchEvent(rawUpdate('pointerrawupdate', 3, 0));
    target.dispatchEvent(rawUpdate('pointerrawupdate', 5, 0));
    expect(moves).toEqual([]);
    c.tasks();
    expect(moves).toEqual([5]);
  });

  it('applies a high-rate device at most once per interval, newest wins, and never loses the last update', () => {
    const target = new EventTarget();
    const c = clock();
    const moves: number[] = [];
    const drag = rawDragMoves({ clientX: 0, clientY: 0 }, (dx) => moves.push(dx), target, c);
    // A 1000 Hz mouse: one update a millisecond for 10 ms.
    for (let i = 1; i <= 10; i++) {
      target.dispatchEvent(rawUpdate('pointerrawupdate', i, 0));
      if (i === 2) expect(drag.pending()).toBe(true);
      c.advance(1);
    }
    // The first at once (in its task), then the newest as each interval ends.
    expect(moves).toEqual([1, 4, 8]);
    expect(moves.length).toBeLessThanOrEqual(Math.ceil(10 / RAW_MOVE_MIN_INTERVAL_MS));
    expect(drag.pending()).toBe(true);
    c.advance(RAW_MOVE_MIN_INTERVAL_MS);
    expect(moves).toEqual([1, 4, 8, 10]);
    expect(drag.pending()).toBe(false);
    expect(drag.updates()).toBe(10);
    expect(drag.applied()).toBe(4);
  });

  it('applies a waiting update when the gesture ends with up, and drops it on cancel', () => {
    const c = clock();
    const upTarget = new EventTarget();
    const upMoves: number[] = [];
    const up = rawDragMoves({ clientX: 0, clientY: 0 }, (dx) => upMoves.push(dx), upTarget, c);
    upTarget.dispatchEvent(rawUpdate('pointerrawupdate', 3, 0));
    c.tasks();
    upTarget.dispatchEvent(rawUpdate('pointerrawupdate', 7, 0));
    up.stop(true);
    expect(upMoves).toEqual([3, 7]);
    c.advance(RAW_MOVE_MIN_INTERVAL_MS);
    expect(upMoves).toEqual([3, 7]);
    expect(c.pending()).toBe(0);

    const cancelTarget = new EventTarget();
    const cancelMoves: number[] = [];
    const cancel = rawDragMoves({ clientX: 0, clientY: 0 }, (dx) => cancelMoves.push(dx), cancelTarget, c);
    cancelTarget.dispatchEvent(rawUpdate('pointerrawupdate', 3, 0));
    c.tasks();
    cancelTarget.dispatchEvent(rawUpdate('pointerrawupdate', 7, 0));
    cancel.stop();
    c.advance(RAW_MOVE_MIN_INTERVAL_MS);
    expect(cancelMoves).toEqual([3]);
  });

  it('ignores other pointers and stops on up or cancel', () => {
    const target = new EventTarget();
    const c = clock();
    const moves: number[] = [];
    const drag = rawDragMoves({ clientX: 0, clientY: 0, pointerId: 7 }, (dx) => moves.push(dx), target, c);
    target.dispatchEvent(rawUpdate('pointerrawupdate', 5, 0, 8));
    target.dispatchEvent(rawUpdate('pointerrawupdate', 6, 0, 7));
    c.tasks();
    drag.stop();
    drag.stop();
    target.dispatchEvent(rawUpdate('pointerrawupdate', 9, 0, 7));
    c.advance(RAW_MOVE_MIN_INTERVAL_MS);
    expect(moves).toEqual([6]);
  });

  it('knows whether a window dispatches raw updates', () => {
    expect(supportsRawUpdates(null)).toBe(false);
    expect(supportsRawUpdates({ onpointerrawupdate: null } as unknown as Window)).toBe(true);
    expect(supportsRawUpdates({} as unknown as Window)).toBe(false);
  });
});
