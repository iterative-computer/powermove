// @vitest-environment happy-dom
import { flushSync, mount, unmount } from 'svelte';
import { expect, it, vi } from 'vitest';
import WorkingTimer from './WorkingTimer.svelte';

it('updates elapsed time across minute and hour boundaries and clears its interval', () => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date('2026-09-06T00:00:00Z'));
  const target = document.createElement('div');
  const instance = mount(WorkingTimer, { target, props: { startedAt: Date.now() - 59000 } });
  flushSync();
  expect(target.textContent).toBe('Working for 59s');
  vi.advanceTimersByTime(1000); flushSync();
  expect(target.textContent).toBe('Working for 1m 0s');
  vi.advanceTimersByTime(3540000); flushSync();
  expect(target.textContent).toBe('Working for 1h 0m 0s');
  unmount(instance); flushSync();
  expect(vi.getTimerCount()).toBe(0);
  vi.useRealTimers();
});
