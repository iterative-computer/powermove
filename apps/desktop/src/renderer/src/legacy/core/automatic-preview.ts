export interface AutomaticPreviewState {
  key: string;
  project: object | null;
  time: number;
  fps: number;
  start: number;
  /** The work area's end is exclusive, as it is during playback. */
  end: number;
  blocked: boolean;
  budget: number;
  bytes: number;
  frameBytes: number;
}

export interface AutomaticPreviewOptions {
  state: () => AutomaticPreviewState;
  has: (time: number) => boolean;
  /** Check valid immediately before committing the completed frame. */
  prepare: (time: number, valid: () => boolean) => Promise<boolean>;
  schedule: (callback: () => void, delayMs: number) => () => void;
  now?: () => number;
}

const IDLE_DELAY = 750;
const MIN_FRAME_DELAY = 50;
const MAX_PREVIEW_FRAMES = 4096;
const FRAME_EPSILON = 1e-7;

function sameWork(a: AutomaticPreviewState | null, b: AutomaticPreviewState): boolean {
  return !!a && a.key === b.key && a.project === b.project && Object.is(a.time, b.time)
    && Object.is(a.fps, b.fps) && Object.is(a.start, b.start) && Object.is(a.end, b.end)
    && a.blocked === b.blocked && Object.is(a.budget, b.budget) && Object.is(a.frameBytes, b.frameBytes);
}

function hasRoom(state: AutomaticPreviewState): boolean {
  return !state.blocked && !!state.project && Number.isFinite(state.time)
    && Number.isFinite(state.fps) && state.fps > 0
    && Number.isFinite(state.start) && Number.isFinite(state.end) && state.end > Math.max(0, state.start)
    && Number.isFinite(state.budget) && Number.isFinite(state.bytes) && state.bytes >= 0
    && Number.isFinite(state.frameBytes) && state.frameBytes > 0
    && state.bytes + Math.ceil(state.frameBytes) <= Math.floor(state.budget);
}

/** Bounded RAM preparation, independent of the renderer and its resources.
 * request() is cheap enough for a paused editor's RAF: unchanged state never
 * scans frames or replaces timers. Rendering/decoding must remain isolated in
 * prepare(), and that callback must check valid() before storing its result. */
export function createAutomaticPreview(options: AutomaticPreviewOptions): {
  request: () => void;
  cancel: () => void;
  dispose: () => void;
} {
  const now = options.now || (() => performance.now());
  let current: AutomaticPreviewState | null = null;
  let generation = 0;
  let idleAt = 0;
  let cursor = 0;
  let lastFrame = -1;
  let failures = 0;
  let pending = false;
  let disposed = false;
  let timer: { cancel: () => void } | null = null;

  const read = () => ({ ...options.state() });
  const stopTimer = () => {
    const previous = timer;
    timer = null;
    previous?.cancel();
  };
  function resetRange(state: AutomaticPreviewState): void {
    const first = Math.max(0, Math.ceil(state.start * state.fps - FRAME_EPSILON));
    const end = Math.ceil(state.end * state.fps - FRAME_EPSILON) - 1;
    cursor = Math.max(first, Math.min(end, Math.floor(state.time * state.fps + FRAME_EPSILON)));
    const frames = Math.min(MAX_PREVIEW_FRAMES, Math.max(1, Math.floor(state.budget / Math.ceil(state.frameBytes))));
    lastFrame = Math.min(end, cursor + frames - 1);
  }
  function update(state: AutomaticPreviewState): boolean {
    const changed = !sameWork(current, state);
    if (changed) {
      generation++;
      stopTimer();
      idleAt = now() + IDLE_DELAY;
      resetRange(state);
      failures = 0;
    } else if (current && state.bytes < current.bytes) {
      // A cache eviction can make a previously visited frame eligible again.
      resetRange(state);
      failures = 0;
    }
    current = state;
    if (!hasRoom(state)) stopTimer();
    return changed;
  }
  function arm(delay: number): void {
    if (disposed || pending || timer || !current || !hasRoom(current) || cursor > lastFrame) return;
    const captured = generation;
    const scheduled = { cancel: () => {} };
    timer = scheduled;
    scheduled.cancel = options.schedule(() => {
      if (disposed || timer !== scheduled || captured !== generation) return;
      timer = null;
      void run();
    }, Math.max(delay, idleAt - now(), 0));
  }
  async function run(): Promise<void> {
    if (disposed || pending) return;
    const changed = update(read());
    if (changed || now() < idleAt) { arm(0); return; }
    if (!current || !hasRoom(current)) return;
    const captured = current;
    const capturedGeneration = generation;
    while (cursor <= lastFrame && options.has(cursor / captured.fps)) cursor++;
    if (cursor > lastFrame) return;
    const frame = cursor;
    const time = frame / captured.fps;
    const valid = () => {
      if (disposed || generation !== capturedGeneration) return false;
      const latest = read();
      return sameWork(captured, latest) && hasRoom(latest);
    };
    if (!valid()) { update(read()); arm(0); return; }
    pending = true;
    const started = now();
    let prepared = false;
    try { prepared = await options.prepare(time, valid); }
    catch { /* Failed decoding/rendering may recover; never leak a rejection. */ }
    finally {
      pending = false;
      if (!disposed) {
        update(read());
        if (generation === capturedGeneration) {
          if (prepared) { if (cursor === frame) cursor = frame + 1; failures = 0; }
          else failures++;
        }
        // Keep at least as much time free as preparation consumed. Transient
        // failures back off progressively instead of monopolizing the editor.
        const cost = generation === capturedGeneration ? Math.max(0, now() - started) : 0;
        const retry = failures ? Math.min(30_000, 1000 * 2 ** Math.min(failures - 1, 5)) : 0;
        arm(Math.max(MIN_FRAME_DELAY, cost, retry));
      }
    }
  }
  function request(): void {
    if (disposed) return;
    update(read());
    arm(MIN_FRAME_DELAY);
  }
  function cancel(): void {
    generation++;
    stopTimer();
    current = null;
  }
  function dispose(): void {
    disposed = true;
    cancel();
  }
  return { request, cancel, dispose };
}
