// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { MAX_DEFER_MS, markMediaActivity, whenMediaIdle } from './activity';

afterEach(() => { vi.useRealTimers(); vi.unstubAllGlobals(); });

describe('whenMediaIdle', () => {
  it('waits for scrubbing to rest, then for an idle callback', async () => {
    vi.useFakeTimers();
    let idle: (() => void) | null = null;
    vi.stubGlobal('requestIdleCallback', (cb: () => void) => { idle = cb; return 1; });
    markMediaActivity();
    let done = false;
    const waiting = whenMediaIdle({ playing: false }).then(() => { done = true; });
    await vi.advanceTimersByTimeAsync(300);
    document.dispatchEvent(new Event('seeking'));
    await vi.advanceTimersByTimeAsync(400);
    expect(idle).toBeNull();
    await vi.advanceTimersByTimeAsync(200);
    expect(idle).not.toBeNull();
    expect(done).toBe(false);
    idle!();
    await waiting;
    expect(done).toBe(true);
  });

  it('never waits longer than the bound while playback loops', async () => {
    vi.useFakeTimers();
    vi.stubGlobal('requestIdleCallback', () => 1);
    const looping = setInterval(() => document.dispatchEvent(new Event('timeupdate')), 250);
    let done = false;
    void whenMediaIdle({ playing: true }).then(() => { done = true; });
    await vi.advanceTimersByTimeAsync(MAX_DEFER_MS - 1);
    expect(done).toBe(false);
    await vi.advanceTimersByTimeAsync(1);
    expect(done).toBe(true);
    clearInterval(looping);
  });
});
