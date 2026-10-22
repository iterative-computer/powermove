import { afterEach, expect, it, vi } from 'vitest';
import { createMemoryRelief, type ReliefTarget } from './memory-relief';

afterEach(() => { vi.useRealTimers(); });

function target(options: { attached?: boolean; fails?: boolean; destroyed?: boolean } = {}) {
  const calls: string[] = [];
  let attached = options.attached ?? false;
  const value: ReliefTarget = {
    isDestroyed: () => options.destroyed ?? false,
    debugger: {
      isAttached: () => attached,
      attach: () => { calls.push('attach'); attached = true; },
      detach: () => { calls.push('detach'); attached = false; },
      sendCommand: async (method, params) => { calls.push(`${method} ${JSON.stringify(params)}`); if (options.fails) throw new Error('busy'); return {}; },
    },
  };
  return { value, calls };
}

it('reports moderate pressure once a window has had no new document for the delay, and leaves its debugger as it was', async () => {
  vi.useFakeTimers();
  const relief = createMemoryRelief({ delayMs: 1000 });
  const a = target(), b = target({ attached: true });
  relief.schedule(a.value); await vi.advanceTimersByTimeAsync(600);
  relief.schedule(a.value); await vi.advanceTimersByTimeAsync(600);
  expect(a.calls).toEqual([]);
  await vi.advanceTimersByTimeAsync(500);
  expect(a.calls).toEqual(['attach', 'Memory.simulatePressureNotification {"level":"moderate"}', 'detach']);
  relief.schedule(b.value); await vi.advanceTimersByTimeAsync(1000);
  expect(b.calls).toEqual(['Memory.simulatePressureNotification {"level":"moderate"}']);
});

it('never fails an open: refused commands, destroyed windows and disposal are quiet', async () => {
  vi.useFakeTimers();
  const warnings: string[] = [];
  const relief = createMemoryRelief({ delayMs: 10, warn: message => warnings.push(message) });
  const failing = target({ fails: true }), gone = target({ destroyed: true }), later = target();
  relief.schedule(failing.value); relief.schedule(gone.value); relief.schedule(later.value);
  await vi.advanceTimersByTimeAsync(10);
  expect(failing.calls.at(-1)).toBe('detach');
  expect(warnings).toEqual(['memory relief skipped: busy']);
  expect(gone.calls).toEqual([]);
  relief.schedule(later.value); relief.dispose(); await vi.advanceTimersByTimeAsync(100);
  expect(later.calls).toHaveLength(3);
});
