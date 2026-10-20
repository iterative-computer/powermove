import { describe, expect, it, vi } from 'vitest';
import { readWithRetry } from './read-retry';

const deadlock = () => Object.assign(new Error('Unknown system error -11, read'), {
  errno: -11, code: 'Unknown system error -11'
});

describe('readWithRetry', () => {
  it('retries a transient macOS deadlock', async () => {
    const read = vi.fn().mockRejectedValueOnce(deadlock()).mockResolvedValue(new Uint8Array([7]));
    await expect(readWithRetry(read)).resolves.toEqual(new Uint8Array([7]));
    expect(read).toHaveBeenCalledTimes(2);
  });

  it('stops after three deadlocks and preserves the original error', async () => {
    const error = deadlock();
    const read = vi.fn().mockRejectedValue(error);
    await expect(readWithRetry(read)).rejects.toBe(error);
    expect(read).toHaveBeenCalledTimes(3);
  });

  it('does not retry a permanent read failure', async () => {
    const error = Object.assign(new Error('permission denied'), { code: 'EACCES' });
    const read = vi.fn().mockRejectedValue(error);
    await expect(readWithRetry(read)).rejects.toBe(error);
    expect(read).toHaveBeenCalledTimes(1);
  });
});
