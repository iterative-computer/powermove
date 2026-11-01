import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process';
import { mkdir } from 'node:fs/promises';
import path from 'node:path';
/** Linux tests use the same POSIX helper; packaged macOS uses the signed binary. */
export class TransitionLock {
  private static executable: string | undefined;
  private released = false;
  private failed = false;
  private constructor(readonly file: string, private child: ChildProcessWithoutNullStreams) {
    child.on('exit', () => { if (!this.released) this.failed = true; });
  }
  static configure(executable: string): void { this.executable = executable; }
  static async acquire(file: string): Promise<TransitionLock> {
    if (!this.executable) throw new Error('The transition lock helper is unavailable.');
    await mkdir(path.dirname(file), { recursive: true });
    const child = spawn(this.executable, [file, String(process.pid)], { stdio: 'pipe' });
    return new Promise((resolve, reject) => {
      let output = '', settled = false;
      const timer = setTimeout(() => fail(), 5000);
      const fail = () => {
        if (settled) return;
        settled = true; clearTimeout(timer); child.kill();
        reject(new Error('This library or project is locked or has uncertain ownership. Your data has been kept.'));
      };
      child.once('error', fail); child.once('exit', fail);
      child.stdout.on('data', chunk => {
        output += String(chunk);
        if (!settled && output === 'ready\n') {
          settled = true; clearTimeout(timer);
          resolve(new TransitionLock(file, child));
        } else if (output.length > 128) fail();
      });
    });
  }
  assertOwnedSync(): void {
    if (this.failed || this.released || this.child.exitCode !== null) throw new Error('The project lock heartbeat failed. Your edits have been kept.');
  }
  async assertOwned(): Promise<void> { this.assertOwnedSync(); }
  async release(): Promise<void> {
    if (this.released) return;
    this.released = true;
    const exited = new Promise<void>((resolve, reject) => {
      if (this.child.exitCode !== null) { resolve(); return; }
      this.child.once('exit', code => code === 0 ? resolve() : reject(new Error('The lock helper stopped unexpectedly.')));
    });
    this.child.stdin.end();
    await exited;
  }
}
export const projectLockPath = (file: string): string => path.join(path.dirname(file), `${path.basename(file, path.extname(file))}.pmvlock`);
