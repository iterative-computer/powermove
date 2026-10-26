import { afterEach, expect, it, vi } from 'vitest';
import { ClosePreparation } from './close-preparation';

afterEach(() => vi.useRealTimers());

it('cancels a stalled phase promptly and consumes its eventual rejection', async () => {
  let reject!: (error: Error) => void;
  const report = vi.fn();
  const close = new ClosePreparation('one', report);
  const promise = close.wait('Waiting for save', () => new Promise((_resolve, fail) => { reject = fail; }));
  await Promise.resolve();
  const rejected = expect(promise).rejects.toThrow('Close cancelled');
  close.cancel();
  await rejected;
  reject(new Error('Late disk failure'));
  await Promise.resolve();
  expect(report).toHaveBeenCalledWith('Waiting for save');
  await expect(close.wait('Next phase', vi.fn())).rejects.toThrow('Close cancelled');
});

it('times out work without authorizing close, but lets the user take time in dialogs', async () => {
  vi.useFakeTimers();
  const close = new ClosePreparation('one', () => {});
  const work = close.wait('Recovery', () => new Promise(() => {}));
  const rejected = expect(work).rejects.toThrow('still open');
  await vi.advanceTimersByTimeAsync(60_000);
  await rejected;
  expect(vi.getTimerCount()).toBe(0);
  const dialog = new ClosePreparation('two', () => {});
  let release!: () => void;
  const decision = dialog.wait('Your decision', () => new Promise<void>(resolve => { release = resolve; }), 0);
  await vi.advanceTimersByTimeAsync(600_000);
  expect(dialog.controller.signal.aborted).toBe(false);
  release(); await decision;
});
