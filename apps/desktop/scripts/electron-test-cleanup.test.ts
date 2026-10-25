import { afterEach, expect, it, vi } from 'vitest';
import { closeTestProcess } from '../e2e/helpers/close';

afterEach(() => vi.useRealTimers());

it('closes a responsive test process without terminating it', async () => {
  const app = { evaluate: vi.fn().mockResolvedValue(null), close: vi.fn().mockResolvedValue(null), process: vi.fn(() => ({ kill: vi.fn() })) };
  expect(await closeTestProcess(app as never, 100)).toBe(false);
  expect(app.close).toHaveBeenCalledOnce();
  expect(app.process).not.toHaveBeenCalled();
});

it.each(['evaluate', 'close'])('terminates a test process when %s never settles, so profile cleanup can proceed', async operation => {
  vi.useFakeTimers();
  const kill = vi.fn();
  const app = { evaluate: vi.fn().mockResolvedValue(null), close: vi.fn().mockResolvedValue(null), process: vi.fn(() => ({ kill })) };
  app[operation as 'evaluate' | 'close'].mockImplementation(() => new Promise(() => {}));
  const closing = closeTestProcess(app as never, 100);
  await vi.advanceTimersByTimeAsync(100);
  expect(await closing).toBe(true);
  expect(kill).toHaveBeenCalledExactlyOnceWith('SIGKILL');
  expect(vi.getTimerCount()).toBe(0);
});
