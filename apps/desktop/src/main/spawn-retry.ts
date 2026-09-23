import { spawn, type ChildProcess, type SpawnOptions } from 'node:child_process';
import path from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';

type SpawnProcess<T extends ChildProcess> = (command: string, args: readonly string[], options: SpawnOptions) => T;

/** macOS can reject posix_spawn with EDEADLK before a child exists. Retrying is
 * safe only in that case: once 'spawn' fires, the child may have done work. */
export async function spawnWithRetry<T extends ChildProcess = ChildProcess>(
  command: string,
  args: readonly string[],
  options: SpawnOptions,
  operation: string,
  spawnProcess: SpawnProcess<T> = spawn as unknown as SpawnProcess<T>
): Promise<T> {
  for (let attempt = 0; attempt < 3; attempt++) {
    try {
      return await new Promise<T>((resolve, reject) => {
        let child: T;
        try { child = spawnProcess(command, args, options); }
        catch (error) { reject(error); return; }
        // Keep the launch listener until the caller can install its own error
        // listener after awaiting this promise.
        child.once('spawn', () => {
          resolve(child);
          setImmediate(() => child.removeListener('error', reject));
        });
        child.once('error', reject);
      });
    } catch (error) {
      const failure = error as NodeJS.ErrnoException;
      const deadlock = failure.errno === -11 || failure.code === 'EDEADLK' || failure.code === 'Unknown system error -11';
      if (deadlock && attempt < 2) { await delay(attempt === 0 ? 100 : 300); continue; }
      const detail = error instanceof Error ? error.message : String(error);
      throw new Error(`Could not start ${operation} (${path.basename(command)}): ${detail}`, { cause: error });
    }
  }
  throw new Error(`Could not start ${operation} (${path.basename(command)}).`);
}
