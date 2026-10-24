// @vitest-environment happy-dom
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { SvelteMap } from 'svelte/reactivity';
import StatusBar from './StatusBar.svelte';
import { frameBus } from '../runtime/frame-bus';
import { performanceMonitor } from '../runtime/performance-monitor';

let instance: ReturnType<typeof mount> | undefined;
beforeEach(() => {
  vi.useFakeTimers(); vi.setSystemTime(0);
  document.body.innerHTML = '<div id="status"></div>';
  performanceMonitor.clear();
});
afterEach(async () => {
  if (instance) await unmount(instance);
  instance = undefined;
  frameBus.clear(); performanceMonitor.clear();
  vi.useRealTimers(); vi.restoreAllMocks();
  document.body.replaceChildren();
});

function status(home = false, settings: { readonly isOpen: boolean } = { isOpen: false }) {
  const listeners = new Map<string, Set<() => void>>();
  const bus = {
    on(event: string, fn: () => void) {
      const set = listeners.get(event) ?? new Set(); listeners.set(event, set); set.add(fn);
      return () => set.delete(fn);
    },
    emit(event: string) { for (const fn of [...listeners.get(event) ?? []]) fn(); },
  };
  const text = vi.fn(() => `Clock ${Date.now()}`);
  const PM: Record<string, any> = {
    bus, proj: { layers: [], w: 320, h: 180, fps: 30 }, app: { dirty: false }, GL: { gl: {} },
    ProjectsScreen: { isOpen: home }, SettingsUI: settings,
    Kernel: { status: { list: () => [{ id: 'clock', text }] } },
    invalidate: vi.fn(() => frameBus.emit('status')),
  };
  instance = mount(StatusBar, { target: document.getElementById('status')!, props: { PM } });
  flushSync();
  return { PM, bus, text, element: document.getElementById('status')! };
}

it('does no periodic status work on Home and refreshes immediately when the editor returns', () => {
  vi.spyOn(document, 'hidden', 'get').mockReturnValue(false);
  const { PM, bus, text, element } = status(true), initialCalls = text.mock.calls.length;
  flushSync(() => vi.advanceTimersByTime(30_000));
  expect(PM.invalidate).not.toHaveBeenCalled(); expect(text).toHaveBeenCalledTimes(initialCalls);
  flushSync(() => { PM.ProjectsScreen.isOpen = false; bus.emit('projects:screen'); });
  expect(element.textContent).toContain('Clock 30000');
  flushSync(() => vi.advanceTimersByTime(2000));
  expect(PM.invalidate).toHaveBeenCalledTimes(2);
  expect(element.textContent).toContain('Clock 32000');
  flushSync(() => { PM.ProjectsScreen.isOpen = true; bus.emit('projects:screen'); });
  PM.invalidate.mockClear(); const hiddenCalls = text.mock.calls.length;
  flushSync(() => vi.advanceTimersByTime(30_000));
  expect(PM.invalidate).not.toHaveBeenCalled(); expect(text).toHaveBeenCalledTimes(hiddenCalls);
});

it('suspends polling while the document is hidden and avoids duplicate timers across visibility changes', () => {
  const hidden = vi.spyOn(document, 'hidden', 'get').mockReturnValue(false);
  const { PM, element } = status();
  for (let cycle = 0; cycle < 3; cycle++) {
    flushSync(() => { hidden.mockReturnValue(true); document.dispatchEvent(new Event('visibilitychange')); });
    PM.invalidate.mockClear();
    flushSync(() => vi.advanceTimersByTime(5000)); expect(PM.invalidate).not.toHaveBeenCalled();
    flushSync(() => { hidden.mockReturnValue(false); document.dispatchEvent(new Event('visibilitychange')); });
    expect(element.textContent).toContain(`Clock ${Date.now()}`);
    PM.invalidate.mockClear();
    flushSync(() => vi.advanceTimersByTime(2000)); expect(PM.invalidate).toHaveBeenCalledTimes(2);
  }
});

it('keeps foreground polling, suspends it in Settings, and releases listeners and timers on unmount', async () => {
  vi.spyOn(document, 'hidden', 'get').mockReturnValue(false);
  const { PM, bus } = status();
  flushSync(() => vi.advanceTimersByTime(2000)); expect(PM.invalidate).toHaveBeenCalledTimes(2);
  flushSync(() => { PM.SettingsUI.isOpen = true; bus.emit('settings:screen'); });
  PM.invalidate.mockClear();
  flushSync(() => vi.advanceTimersByTime(10_000)); expect(PM.invalidate).not.toHaveBeenCalled();
  flushSync(() => { PM.SettingsUI.isOpen = false; bus.emit('settings:screen'); });
  flushSync(() => vi.advanceTimersByTime(2000)); expect(PM.invalidate).toHaveBeenCalledTimes(2);
  await unmount(instance!); instance = undefined; PM.invalidate.mockClear();
  flushSync(() => { bus.emit('projects:screen'); document.dispatchEvent(new Event('visibilitychange')); vi.advanceTimersByTime(5000); });
  expect(PM.invalidate).not.toHaveBeenCalled(); expect(vi.getTimerCount()).toBe(0);
});

it('expires stale performance warnings immediately on return without waiting for another polling tick', () => {
  let now = 0;
  vi.spyOn(performance, 'now').mockImplementation(() => now);
  const hidden = vi.spyOn(document, 'hidden', 'get').mockReturnValue(false);
  performanceMonitor.record({ id: 'shader:slow', name: 'Slow shader', kind: 'shader' }, 60, now);
  const { PM, element } = status();
  expect(element.textContent).toContain('Performance');
  flushSync(() => { hidden.mockReturnValue(true); document.dispatchEvent(new Event('visibilitychange')); });
  PM.invalidate.mockClear(); now = 20_000;
  flushSync(() => vi.advanceTimersByTime(20_000)); expect(PM.invalidate).not.toHaveBeenCalled();
  flushSync(() => { hidden.mockReturnValue(false); document.dispatchEvent(new Event('visibilitychange')); });
  expect(element.textContent).not.toContain('Performance');
});

it('keeps timer ownership stable when Settings visibility comes from a Svelte rune', () => {
  vi.spyOn(document, 'hidden', 'get').mockReturnValue(false);
  const shown = new SvelteMap([['open', false]]);
  const interval = vi.spyOn(window, 'setInterval');
  const { PM, bus } = status(false, { get isOpen() { return !!shown.get('open'); } });
  expect(interval).toHaveBeenCalledTimes(1);
  flushSync(() => { shown.set('open', true); bus.emit('settings:screen'); });
  flushSync(() => vi.advanceTimersByTime(5000)); expect(PM.invalidate).not.toHaveBeenCalled();
  flushSync(() => { shown.set('open', false); bus.emit('settings:screen'); });
  expect(interval).toHaveBeenCalledTimes(2);
  flushSync(() => vi.advanceTimersByTime(2000)); expect(PM.invalidate).toHaveBeenCalledTimes(2);
  expect(interval).toHaveBeenCalledTimes(2);
});
