import { describe, expect, it, vi } from 'vitest';
import { retryResync } from './resync-retry';

const busy = (ms: number) => Object.assign(new Error('reader busy'), { error: { retry: { kind: 'after', ms } } });

describe('resync admission retry', () => {
  it('retains the same request while waiting for reader capacity', async () => {
    vi.useFakeTimers();
    try {
      const send = vi.fn().mockRejectedValueOnce(busy(50)).mockRejectedValueOnce(busy(50)).mockResolvedValue('applied');
      const result = retryResync(send);
      await vi.advanceTimersByTimeAsync(100);
      expect(await result).toBe('applied');
      expect(send).toHaveBeenCalledTimes(3);
      expect(vi.getTimerCount()).toBe(0);
    } finally { vi.useRealTimers(); }
  });
  it('preserves permanent failures', async () => {
    const send = vi.fn().mockRejectedValue(new Error('invalid hash'));
    await expect(retryResync(send)).rejects.toThrow('invalid hash');
    expect(send).toHaveBeenCalledOnce();
  });
  it('cancels the delay when the connection closes', async () => {
    vi.useFakeTimers();
    try {
      const controller = new AbortController();
      const send = vi.fn().mockRejectedValue(busy(250));
      const result = expect(retryResync(send, controller.signal)).rejects.toThrow('connection closed');
      await vi.advanceTimersByTimeAsync(0);
      controller.abort();
      await result;
      expect(vi.getTimerCount()).toBe(0);
      expect(send).toHaveBeenCalledOnce();
    } finally { vi.useRealTimers(); }
  });
});
