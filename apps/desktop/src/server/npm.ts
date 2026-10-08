import { execFile } from 'node:child_process';
import { existsSync } from 'node:fs';
import path from 'node:path';
import { promisify } from 'node:util';
const run = promisify(execFile);

/** Windows npm.cmd cannot be execFile'd. Run its real JS entry with Node,
 * retaining literal arguments (including paths with spaces or metacharacters). */
export async function runNpm(args: string[], options: { timeout?: number; maxBuffer?: number } = {}): Promise<void> {
  if (process.platform !== 'win32') { await run('npm', args, options); return; }
  const { stdout } = await run('where.exe', ['npm.cmd'], { windowsHide: true });
  const candidates = stdout.trim().split(/\r?\n/).map(file => path.join(path.dirname(file), 'node_modules', 'npm', 'bin', 'npm-cli.js'));
  const cli = candidates.find(file => existsSync(file));
  if (!cli) throw new Error('npm is unavailable. Install Node.js 22 or newer, then retry.');
  await run(process.execPath, [cli, ...args], { ...options, windowsHide: true });
}
