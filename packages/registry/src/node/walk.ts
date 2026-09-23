import { lstat, readdir, readFile } from 'node:fs/promises';
import { join } from 'node:path';
import { isExcludedPath, snapshot, SnapshotError, type SnapshotInput } from '../snapshot';

export async function walkDir(dir: string): Promise<SnapshotInput[]> {
  const files: SnapshotInput[] = [];
  async function walk(absolute: string, relative: string): Promise<void> {
    const entries = await readdir(absolute);
    for (const name of entries) {
      const path = relative ? `${relative}/${name}` : name;
      if (isExcludedPath(path)) continue;
      const child = join(absolute, name);
      const stat = await lstat(child);
      if (stat.isSymbolicLink()) throw new SnapshotError('symlink', path);
      if (stat.isDirectory()) await walk(child, path);
      else if (stat.isFile()) files.push({ path, bytes: new Uint8Array(await readFile(child)) });
      else throw new SnapshotError('not_regular', path);
    }
  }
  await walk(dir, '');
  return files;
}

export async function snapshotDir(dir: string) { return snapshot(await walkDir(dir)); }
