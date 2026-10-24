/**
 * Background media work (editing previews, engine poster persistence) waits
 * until nothing is scrubbing or playing, then for an idle period, so the UI
 * updates it publishes never land inside an interactive frame.
 *
 * Activity is any media element seek, play or time update (captured at the
 * document: media events do not bubble), timeline playback (`PM.playing`),
 * and explicit marks from surfaces without a seeking element (the native
 * source monitor).
 */
const QUIET_MS = 500;
const IDLE_TIMEOUT_MS = 1_000;
/** Background publication never waits longer than this, even while a
 * source keeps playing or a timeline loops. */
export const MAX_DEFER_MS = 2_000;
let last = -Infinity;
let listening = false;

export function markMediaActivity(): void {
  last = performance.now();
}

function listen(): void {
  if (listening || typeof document === 'undefined' || typeof document.addEventListener !== 'function') return;
  listening = true;
  for (const type of ['seeking', 'playing', 'timeupdate']) document.addEventListener(type, markMediaActivity, true);
}

const sleep = (ms: number) => new Promise<void>(resolve => setTimeout(resolve, ms));

/** Resolves once media has been quiet for `quietMs` and the renderer is
 * idle, or after `MAX_DEFER_MS` whatever is playing. */
export async function whenMediaIdle(PM: { playing?: boolean } | null | undefined, quietMs = QUIET_MS): Promise<void> {
  listen();
  const deadline = performance.now() + MAX_DEFER_MS;
  for (;;) {
    const left = deadline - performance.now();
    if (left <= 0) return;
    const wait = PM?.playing ? quietMs : last + quietMs - performance.now();
    if (wait <= 0) break;
    await sleep(Math.min(wait, left));
  }
  const idle = (globalThis as { requestIdleCallback?: (cb: () => void, options?: { timeout: number }) => number }).requestIdleCallback;
  const timeout = Math.max(1, Math.min(IDLE_TIMEOUT_MS, deadline - performance.now()));
  await new Promise<void>(resolve => idle ? idle(() => resolve(), { timeout }) : setTimeout(resolve, 0));
}
