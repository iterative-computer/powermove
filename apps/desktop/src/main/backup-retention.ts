import { constants } from 'node:fs';
import { lstat, open, readdir, unlink } from 'node:fs/promises';
import path from 'node:path';
import { PROJECT_ID } from '../shared/ipc';
import { hashHandle, sameFile } from './durable-file';

const DAY = 24 * 60 * 60 * 1000;
const BACKUP_NAME = /^(\d{4}-\d{2}-\d{2}T\d{2}-\d{2}-\d{2}-\d{3}Z)\.pmv(?:-(\d+))?$/;
export const DEFAULT_BACKUP_BUDGET = 5 * 1024 ** 3;

type Backup = { file: string; size: number; timestamp: string; collision: number; ino: number; mtime: number; ctime: number };
export type BackupPlan = { remove: Backup[]; totalBytes: number; reclaimableBytes: number; retainedBytes: number };

async function directory(file: string): Promise<boolean> {
  try { return (await lstat(file)).isDirectory(); }
  catch (error: any) { if (error.code === 'ENOENT') return false; throw error; }
}

/** Only app-created generations are eligible. Recovery files, manual copies,
 * symlinks and the two newest generations of every project are preserved. */
export async function planBackupRetention(root: string, budget = DEFAULT_BACKUP_BUDGET, now = Date.now()): Promise<BackupPlan> {
  const remove: Backup[] = [], optional: Backup[] = [];
  let totalBytes = 0;
  if (!await directory(root)) return { remove, totalBytes, reclaimableBytes: 0, retainedBytes: 0 };
  for (const project of await readdir(root, { withFileTypes: true })) {
    if (!project.isDirectory() || !PROJECT_ID.test(project.name)) continue;
    const folder = path.join(root, project.name);
    if (!await directory(folder)) continue;
    const backups: Backup[] = [];
    for (const entry of await readdir(folder, { withFileTypes: true })) {
      const match = BACKUP_NAME.exec(entry.name);
      if (!entry.isFile() || !match) continue;
      const file = path.join(folder, entry.name);
      try {
        const info = await lstat(file);
        if (!info.isFile()) continue;
        backups.push({ file, size: info.size, timestamp: match[1]!, collision: Number(match[2] ?? 0),
          ino: info.ino, mtime: info.mtimeMs, ctime: info.ctimeMs });
        totalBytes += info.size;
      } catch (error: any) { if (error.code !== 'ENOENT') throw error; }
    }
    backups.sort((a, b) => b.timestamp.localeCompare(a.timestamp) || b.collision - a.collision);
    for (let i = 2; i < backups.length; i++) {
      const backup = backups[i]!;
      // mtime may be inherited by a snapshot; use its creation timestamp.
      const created = Date.parse(backup.timestamp.replace(/T(\d{2})-(\d{2})-(\d{2})-(\d{3})Z$/, 'T$1:$2:$3.$4Z'));
      if (i >= 5 || created < now - 30 * DAY) remove.push(backup);
      else optional.push(backup);
    }
  }
  let reclaimableBytes = remove.reduce((sum, item) => sum + item.size, 0);
  optional.sort((a, b) => a.timestamp.localeCompare(b.timestamp) || a.collision - b.collision);
  for (const backup of optional) {
    if (totalBytes - reclaimableBytes <= budget) break;
    remove.push(backup);
    reclaimableBytes += backup.size;
  }
  return { remove, totalBytes, reclaimableBytes, retainedBytes: totalBytes - reclaimableBytes };
}

/** Recheck identity and every parent before deletion; never follow a link or
 * delete a file that changed after the estimate. Call under the save queue. */
export async function applyBackupRetention(root: string, plan: BackupPlan): Promise<number> {
  let freed = 0;
  if (!await directory(root)) return freed;
  for (const backup of plan.remove) {
    const relative = path.relative(root, backup.file);
    if (relative.startsWith('..') || path.isAbsolute(relative) || relative.split(path.sep).length !== 2) continue;
    if (!await directory(path.dirname(backup.file))) continue;
    try {
      const info = await lstat(backup.file);
      if (!info.isFile() || info.ino !== backup.ino || info.size !== backup.size
        || info.mtimeMs !== backup.mtime || info.ctimeMs !== backup.ctime) continue;
      await unlink(backup.file);
      freed += backup.size;
    } catch (error: any) { if (error.code !== 'ENOENT') throw error; }
  }
  return freed;
}

/** The upgrade has no authority to discard unique earlier edits. Retire only
 * exact duplicate successful-save generations, keeping the newest copy.
 * Uncertain saves, manually named files and links are never eligible. */
export async function deduplicateBackups(root: string): Promise<number> {
  if (!await directory(root)) return 0;
  let freed = 0;
  for (const project of await readdir(root, { withFileTypes: true })) {
    if (!project.isDirectory() || !PROJECT_ID.test(project.name)) continue;
    const folder = path.join(root, project.name);
    if (!await directory(folder)) continue;
    const names = (await readdir(folder)).filter(name => BACKUP_NAME.test(name)).sort((a, b) => b.localeCompare(a, undefined, { numeric: true }));
    const kept = new Map<string, string>();
    for (const name of names) {
      if (!await directory(root) || !await directory(folder)) break;
      const file = path.join(folder, name), info = await lstat(file);
      if (!info.isFile()) continue;
      const handle = await open(file, constants.O_RDONLY | constants.O_NOFOLLOW);
      try {
        if (!sameFile(info, await handle.stat())) continue;
        const hash = (await hashHandle(handle)).digest('hex');
        if (!sameFile(info, await handle.stat())) continue;
        const canonical = kept.get(hash);
        if (!canonical) { kept.set(hash, file); continue; }
        // Re-prove that the retained copy exists and matches immediately
        // before deleting a duplicate. A changed file is always kept.
        const retainedInfo = await lstat(canonical);
        if (!retainedInfo.isFile() || retainedInfo.size !== info.size) continue;
        const retained = await open(canonical, constants.O_RDONLY | constants.O_NOFOLLOW);
        try {
          if (!sameFile(retainedInfo, await retained.stat()) || (await hashHandle(retained)).digest('hex') !== hash
            || !sameFile(retainedInfo, await retained.stat()) || !sameFile(retainedInfo, await lstat(canonical))
            || !sameFile(info, await lstat(file)) || !await directory(root) || !await directory(folder)) continue;
          await unlink(file); freed += info.size;
        } finally { await retained.close(); }
      } finally { await handle.close(); }
    }
  }
  return freed;
}
