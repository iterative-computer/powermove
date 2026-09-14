import { spawn, type ChildProcess, type SpawnOptions } from 'node:child_process';

/** posix_spawn itself is synchronous, even though the child runs asynchronously.
 * Launch the small system shell first so loading/verifying a cold Claude binary
 * happens in the child rather than blocking Electron's native event loop.
 * exec preserves its PID for cancellation. Arguments are passed literally; no
 * user shell profile or command interpolation is involved. */
export function spawnClaudeProcess(command: string, args: readonly string[], options: SpawnOptions): ChildProcess {
  return spawn('/bin/sh', ['-c', 'exec "$@"', 'powermove-claude', command, ...args], options);
}
