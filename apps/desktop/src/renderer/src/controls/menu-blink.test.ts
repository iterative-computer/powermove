// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { blink } from './menu-blink';

afterEach(() => { vi.useRealTimers(); vi.unstubAllGlobals(); });
describe('custom menu selection', () => {
  it.each([false, true])('commits synchronously with reduced motion %s, without pending timers', (reduced) => {
    vi.useFakeTimers();
    vi.stubGlobal('matchMedia', () => ({ matches: reduced }));
    const row = document.createElement('div');
    const done = vi.fn();
    blink(row, done);
    expect(done).toHaveBeenCalledOnce();
    expect(row.dataset.blink).toBeUndefined();
    expect(vi.getTimerCount()).toBe(0);
  });
});
