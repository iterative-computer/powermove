// @vitest-environment happy-dom
import { expect, it, vi } from 'vitest';
import { horizontalScrub } from './horizontal-scrub';
it('groups horizontal movement, preserves vertical scrolling, and cancels on disposal', () => {
  vi.useFakeTimers();
  const node = document.createElement('div');
  const options = { begin: vi.fn(), move: vi.fn(), commit: vi.fn(), cancel: vi.fn() };
  const action = horizontalScrub(node, options);
  const wheel = (x: number, y: number) => { const e = new WheelEvent('wheel', { deltaX: x, deltaY: y, cancelable: true }); node.dispatchEvent(e); return e; };
  expect(wheel(1, 20).defaultPrevented).toBe(false);
  expect(options.begin).not.toHaveBeenCalled();
  expect(wheel(20, 1).defaultPrevented).toBe(true);
  expect(wheel(0, 20).defaultPrevented).toBe(true);
  wheel(-10, 0);
  expect(options.begin).toHaveBeenCalledTimes(1);
  expect(options.move).toHaveBeenLastCalledWith(-2.5, expect.anything());
  vi.advanceTimersByTime(350);
  expect(options.commit).toHaveBeenCalledTimes(1);
  wheel(10, 0); action.destroy();
  vi.runAllTimers();
  expect(options.cancel).toHaveBeenCalledTimes(1);
  expect(options.commit).toHaveBeenCalledTimes(1);
  vi.useRealTimers();
});

it('paces haptics and stays quiet when the value cannot change', () => {
  vi.useFakeTimers();
  const alignment = vi.fn();
  Object.defineProperty(window, 'powermove', { configurable: true, value: { haptic: { alignment } } });
  const node = document.createElement('div');
  const move = vi.fn(() => true);
  const action = horizontalScrub(node, { begin() {}, move, commit() {}, cancel() {} });
  const wheel = () => node.dispatchEvent(new WheelEvent('wheel', { deltaX: 24 }));
  wheel(); wheel();
  expect(alignment).toHaveBeenCalledTimes(1);
  vi.advanceTimersByTime(110); wheel();
  expect(alignment).toHaveBeenCalledTimes(2);
  move.mockReturnValue(false);
  vi.advanceTimersByTime(110); wheel();
  expect(alignment).toHaveBeenCalledTimes(2);
  action.destroy();
  delete (window as any).powermove;
  vi.useRealTimers();
});
