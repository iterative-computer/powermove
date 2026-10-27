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
 * Raw updates come at the device's rate, not the display's: a 1000 Hz
 * mouse would run the gesture's move (a document edit and a viewer state
 * for a layer drag) eight times a refresh at 120 Hz. Moves are therefore
 * applied at most once per `minIntervalMs` (4 ms: two a refresh at
 * 120 Hz); updates inside the interval collapse into the newest, applied
 * when the interval ends (or when the gesture ends with `up`, so the final
 * position always lands).
 *
 * A move never runs inside the input event. The engine's frames reach the
 * page as port messages (the presentation worker gets each one through the
 * page's main thread, and its releases go back the same way), and Chromium
 * runs input ahead of every other task: a drag whose moves ran in its raw
 * updates (a document edit, a flush to the engine and the frame's raster
 * answers, 5 to 12 ms each on the real project) kept the main thread in
 * input tasks for the whole gesture, so the engine's frames and releases
 * waited behind it, the engine ran out of free surfaces, and the viewer
 * showed nothing new until the pointer stopped (0 frames in a 1 s drag,
 * pointer to pixels 0.6 to 2.3 s). The update only records the newest
 * pointer; the move runs in an ordinary task posted right behind it (a
 * message, never a clamped timer), in turn with the engine's messages, and
 * the updates that arrive while it waits collapse into the newest.
 *
 * Only moves change source: the drag helper (`api.ui.drag`) still owns
 * pointer capture, the up and cancel paths, and the cursor; its own
 * rAF-aligned moves are ignored while raw updates drive the gesture. Drags
 * that use pointer lock (`infinite`) keep `pointermove`, because raw updates
 * carry no lock deltas.
 */

export type DragMove = (dx: number, dy: number, event: PointerEvent) => void;

/** Fewest milliseconds between two applied raw moves (see above). */
export const RAW_MOVE_MIN_INTERVAL_MS = 4;

/** Whether `view` dispatches `pointerrawupdate` (secure contexts in Chromium). */
export function supportsRawUpdates(view: (Window & typeof globalThis) | Window | null | undefined): boolean {
  return !!view && 'onpointerrawupdate' in view;
}

export type RawDrag = {
  /** The move to give the drag helper: ignores its `pointermove` moves. */
  readonly move: DragMove;
  /** Raw updates seen. */
  readonly updates: () => number;
  /** Moves applied (updates minus the ones collapsed into a newer one). */
  readonly applied: () => number;
  /** Whether the newest update waits for the interval to end (not applied yet). */
  readonly pending: () => boolean;
  /**
   * Stops listening. `flush` applies a waiting update first (on up, so the
   * gesture ends where the pointer did); a cancel drops it.
   */
  stop(flush?: boolean): void;
};

export type RawDragOptions = {
  minIntervalMs?: number;
  now?: () => number;
  timers?: { set(fn: () => void, ms: number): number; clear(id: number): void };
  /** Runs `fn` in an ordinary task right behind the current one (default: a message on a channel). */
  task?: (fn: () => void) => void;
};

/** Ordinary tasks right behind the current one: messages on a channel (timers are clamped). */
function messageTasks(): { post(fn: () => void): void; close(): void } {
  const channel = new MessageChannel();
  const queue: Array<() => void> = [];
  channel.port1.onmessage = () => queue.shift()?.();
  return {
    post(fn) {
      queue.push(fn);
      channel.port2.postMessage(null);
    },
    close() {
      queue.length = 0;
      channel.port1.close();
      channel.port2.close();
    }
  };
}

/**
 * Drives `move` from the pointer's raw updates on `target` (the window
 * hosting the gesture) with the same deltas the drag helper computes
 * (client position minus the press position).
 */
export function rawDragMoves(
  press: { clientX: number; clientY: number; pointerId?: number },
  move: DragMove,
  target: EventTarget,
  options: RawDragOptions = {}
): RawDrag {
  const minInterval = options.minIntervalMs ?? RAW_MOVE_MIN_INTERVAL_MS;
  const now = options.now ?? (() => performance.now());
  const timers = options.timers ?? { set: (fn, ms) => setTimeout(fn, ms) as unknown as number, clear: (id) => clearTimeout(id) };
  const tasks = options.task ? null : messageTasks();
  const task = options.task ?? tasks!.post;
  let count = 0;
  let appliedCount = 0;
  let stopped = false;
  let lastApplied = -Infinity;
  let waiting: PointerEvent | null = null;
  /** A task or an interval timer is due to apply `waiting`. */
  let scheduled = false;
  let timer = 0;
  const apply = (pointer: PointerEvent): void => {
    lastApplied = now();
    appliedCount++;
    move(pointer.clientX - press.clientX, pointer.clientY - press.clientY, pointer);
  };
  const run = (): void => {
    scheduled = false;
    timer = 0;
    if (stopped || !waiting) return;
    const wait = lastApplied + minInterval - now();
    if (wait > 0) {
      // Inside the interval: the newest when it ends.
      scheduled = true;
      timer = timers.set(run, wait);
      return;
    }
    const pointer = waiting;
    waiting = null;
    apply(pointer);
  };
  const onRaw = (event: Event): void => {
    const pointer = event as PointerEvent;
    if (stopped || (press.pointerId !== undefined && pointer.pointerId !== undefined && pointer.pointerId !== press.pointerId)) return;
    count++;
    waiting = pointer;
    if (scheduled) return;
    scheduled = true;
    task(run);
  };
  target.addEventListener('pointerrawupdate', onRaw, true);
  return {
    move: () => {},
    updates: () => count,
    applied: () => appliedCount,
    pending: () => waiting !== null,
    stop(flush = false) {
      if (stopped) return;
      if (timer) timers.clear(timer);
      timer = 0;
      scheduled = false;
      tasks?.close();
      if (flush && waiting) apply(waiting);
      waiting = null;
      stopped = true;
      target.removeEventListener('pointerrawupdate', onRaw, true);
    }
  };
}
