import { afterEach, expect, it, vi } from 'vitest';
import type { BrowserWindow } from 'electron';
import { createEditorClosePreparation } from './editor-close';

afterEach(() => vi.useRealTimers());
const windowFixture = (executeJavaScript: (code: string) => Promise<unknown> = vi.fn(async () => true)) => ({
  isDestroyed: () => false,
  webContents: { isDestroyed: () => false, isCrashed: () => false, executeJavaScript },
}) as unknown as BrowserWindow;

it('shares overlapping Close and Quit and checks cancellation again before unload', async () => {
  let release!: (result: boolean) => void;
  const execute = vi.fn((_code: string) => new Promise<boolean>(resolve => { release = resolve; }));
  const window = windowFixture(execute);
  const close = createEditorClosePreparation(() => true);
  const first = close.prepare(window), second = close.prepare(window);
  expect(second).toBe(first);
  await Promise.resolve();
  expect(execute).toHaveBeenCalledOnce();
  release(true); await first;
  execute.mockResolvedValue(false);
  await expect(close.commit([window])).rejects.toThrow('Close cancelled');
  close.cancelAll();
  await Promise.resolve();
  expect(execute.mock.calls.at(-1)?.[0]).toContain('cancelClosePreparation');
});

it('never authorizes a late success after the native watchdog expires', async () => {
  vi.useFakeTimers();
  let release!: (result: boolean) => void;
  const execute = vi.fn().mockImplementationOnce(() => new Promise<boolean>(resolve => { release = resolve; })).mockResolvedValue(true);
  const window = windowFixture(execute);
  const close = createEditorClosePreparation(() => true);
  const first = close.prepare(window);
  const rejected = expect(first).rejects.toThrow('timed out');
  await vi.advanceTimersByTimeAsync(120_000); await rejected;
  release(true); await Promise.resolve();
  await expect(close.commit([window])).rejects.toThrow('Close cancelled');
  await close.prepare(window);
  await expect(close.commit([window])).resolves.toBeUndefined();
  expect(vi.getTimerCount()).toBe(0);
});

it('releases preparation after a synchronous renderer failure so Close can be retried', async () => {
  const execute = vi.fn().mockImplementationOnce(() => { throw new Error('Renderer unavailable'); }).mockResolvedValue(true);
  const window = windowFixture(execute);
  const close = createEditorClosePreparation(() => true);
  await expect(close.prepare(window)).rejects.toThrow('Renderer unavailable');
  await close.prepare(window);
  await expect(close.commit([window])).resolves.toBeUndefined();
});
