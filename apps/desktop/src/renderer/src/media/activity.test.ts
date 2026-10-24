// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { markMediaActivity, whenMediaIdle } from './activity';

afterEach(() => { vi.useRealTimers(); vi.unstubAllGlobals(); });

describe('whenMediaIdle', () => {
  it('waits for scrubbing and playback to stop before an idle callback', async () => {
    vi.useFakeTimers();
    let idle: (() => void) | null = null;
    vi.stubGlobal('requestIdleCallback', (cb: () => void) => { idle = cb; return 1; });
    const PM = { playing: false };
    markMediaActivity();
    let done = false;
    const waiting = whenMediaIdle(PM).then(() => { done = true; });
    await vi.advanceTimersByTimeAsync(300);
    document.dispatchEvent(new Event('seeking'));
    await vi.advanceTimersByTimeAsync(400);
    expect(idle).toBeNull();
    PM.playing = true;
    await vi.advanceTimersByTimeAsync(2_000);
    expect(idle).toBeNull();
    PM.playing = false;
    await vi.advanceTimersByTimeAsync(600);
    expect(idle).not.toBeNull();
    expect(done).toBe(false);
    idle!();
    await waiting;
    expect(done).toBe(true);
  });
});
