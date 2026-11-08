import { setTimeout as delay } from 'node:timers/promises';

/** Cloud providers and network volumes can temporarily reject a read. Retry
 * the same offset only after rejection, never after a partial successful read.
 * Keep the original error so callers can distinguish unavailability from damage. */
export async function readWithRetry<T>(read: () => Promise<T>): Promise<T> {
  for (let attempt = 0; ; attempt++) {
    try { return await read(); }
    catch (error) {
      const failure = error as NodeJS.ErrnoException;
      const deadlock = failure.errno === -11 || failure.code === 'EDEADLK' || failure.code === 'Unknown system error -11';
      const transient = deadlock || ['EAGAIN', 'ETIMEDOUT', 'ENOTCONN'].includes(failure.code ?? '');
      if (!transient || attempt === 2) throw error;
      await delay(attempt === 0 ? 100 : 300);
    }
  }
}
