import { execFileSync } from 'node:child_process';

/** Opt-in measurement runs only (they build and launch repeatedly on a shared
 * Mac): check the data volume immediately before each launch. Ordinary e2e
 * tests never call this. */
export function requireEngineDiskSpace(): void {
  const output = execFileSync('df', ['-k', '/System/Volumes/Data'], { encoding: 'utf8' });
  console.log(output.trim());
  const row = output.trim().split('\n').at(-1)!.trim().split(/\s+/);
  const available = Number(row[3]) * 1024;
  if (!Number.isFinite(available) || available < 40 * 1024 ** 3) throw new Error('Engine fixups paused: less than 40 GiB available; remove only this worktree engine/target before retrying.');
}
