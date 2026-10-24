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

/** Resolves once media has been quiet for `quietMs` and the renderer is idle. */
export async function whenMediaIdle(PM: { playing?: boolean } | null | undefined, quietMs = QUIET_MS): Promise<void> {
  listen();
  for (;;) {
    const wait = last + quietMs - performance.now();
    if (PM?.playing) await sleep(quietMs);
    else if (wait > 0) await sleep(wait);
    else break;
  }
  const idle = (globalThis as { requestIdleCallback?: (cb: () => void, options?: { timeout: number }) => number }).requestIdleCallback;
  await new Promise<void>(resolve => idle ? idle(() => resolve(), { timeout: IDLE_TIMEOUT_MS }) : setTimeout(resolve, 0));
}
