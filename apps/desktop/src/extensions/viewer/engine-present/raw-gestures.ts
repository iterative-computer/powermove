/*
 * Engine-path gestures read `pointerrawupdate` (amendment 18, step 2 of
 * closing the presentation gap; docs/engine/bridge.md).
 *
 * `pointermove` is aligned to animation frames: Chromium holds it until the
 * next BeginFrame, so a drag handler that changes the view or the document
 * runs inside the rAF that is already presenting. The WebGL viewer draws in
 * that same rAF, so it does not mind; the engine renders in its own process
 * after it hears about the change, so its frame misses that rAF and shows
 * one to two refreshes later (measured: 33 to 36 ms against 23 to 29 ms
 * pointer-to-pixel idle). `pointerrawupdate` is dispatched as soon as the
 * input arrives, so the engine starts rendering up to a refresh earlier.
 *
 * Only moves change source: the drag helper (`api.ui.drag`) still owns
 * pointer capture, the up and cancel paths, and the cursor; its own
 * rAF-aligned moves are ignored while raw updates drive the gesture. Drags
 * that use pointer lock (`infinite`) keep `pointermove`, because raw updates
 * carry no lock deltas.
 */

export type DragMove = (dx: number, dy: number, event: PointerEvent) => void;

/** Whether `view` dispatches `pointerrawupdate` (secure contexts in Chromium). */
export function supportsRawUpdates(view: (Window & typeof globalThis) | Window | null | undefined): boolean {
  return !!view && 'onpointerrawupdate' in view;
}

export type RawDrag = {
  /** The move to give the drag helper: ignores its `pointermove` moves. */
  readonly move: DragMove;
  /** Raw updates seen. */
  readonly updates: () => number;
  /** Stops listening (on up and cancel). */
  stop(): void;
};

/**
 * Drives `move` from the pointer's raw updates on `target` (the window
 * hosting the gesture) with the same deltas the drag helper computes
 * (client position minus the press position).
 */
export function rawDragMoves(press: { clientX: number; clientY: number; pointerId?: number }, move: DragMove, target: EventTarget): RawDrag {
  let count = 0;
  let stopped = false;
  const onRaw = (event: Event): void => {
    const pointer = event as PointerEvent;
    if (stopped || (press.pointerId !== undefined && pointer.pointerId !== undefined && pointer.pointerId !== press.pointerId)) return;
    count++;
    move(pointer.clientX - press.clientX, pointer.clientY - press.clientY, pointer);
  };
  target.addEventListener('pointerrawupdate', onRaw, true);
  return {
    move: () => {},
    updates: () => count,
    stop() {
      if (stopped) return;
      stopped = true;
      target.removeEventListener('pointerrawupdate', onRaw, true);
    }
  };
}
