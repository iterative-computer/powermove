/*
 * Retained imperative event bus for the handful of events that must NOT route
 * through Svelte's reactive scheduler:
 *
 *  overlay        — emitted inside the frame loop right after GL.render; the
 *                   viewer's 2D overlay must draw in the same frame, after GL.
 *  timeline       — 60 Hz playhead repaint of the timeline canvas.
 *  layout-applied — dock/splitter geometry settled; canvas panels re-measure.
 *                   Fires at rAF cadence during splitter drags (keep the
 *                   immediate / +1 rAF / +120 ms resize choreography).
 *  layout         — theme tokens changed; consumers re-read getComputedStyle.
 *  status         — ≤2 Hz perf/status refresh (reads non-reactive GL.stats).
 *  quality        — adaptive quality changed inside the render loop (FBO pool
 *                   teardown); engine-owned.
 *
 * Rule: runes may not be written from inside the render section of the frame
 * loop; these events are the sanctioned channel for that phase.
 */
export type FrameEvent = 'overlay' | 'timeline' | 'layout-applied' | 'layout' | 'status' | 'quality';

type Listener = (payload?: unknown) => void;

const listeners = new Map<FrameEvent, Set<Listener>>();

export const frameBus = {
  on(event: FrameEvent, fn: Listener): () => void {
    let set = listeners.get(event);
    if (!set) listeners.set(event, (set = new Set()));
    set.add(fn);
    return () => {
      set.delete(fn);
    };
  },
  emit(event: FrameEvent, payload?: unknown): void {
    const set = listeners.get(event);
    if (!set) return;
    for (const fn of [...set]) {
      try {
        fn(payload);
      } catch (error) {
        console.error('[frame-bus]', event, error);
      }
    }
  },
  /** Test seam. */
  clear(): void {
    listeners.clear();
  }
};
