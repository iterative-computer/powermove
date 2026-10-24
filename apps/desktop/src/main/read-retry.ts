import { setTimeout as delay } from 'node:timers/promises';

/** macOS can transiently reject a file read with EDEADLK (-11). Retrying the
 * same offset is safe because a failed read has not returned any bytes. */
export async function readWithRetry<T>(read: () => Promise<T>): Promise<T> {
  for (let attempt = 0; ; attempt++) {
    try { return await read(); }
    catch (error) {
      const failure = error as NodeJS.ErrnoException;
      const deadlock = failure.errno === -11 || failure.code === 'EDEADLK' || failure.code === 'Unknown system error -11';
      if (!deadlock || attempt === 2) throw error;
      await delay(attempt === 0 ? 100 : 300);
    }
  }
}
