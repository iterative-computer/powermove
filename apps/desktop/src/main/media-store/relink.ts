/*
 * Automatic relinking of moved or renamed linked originals (plan section
 * 21, "Relinking"): only by content hash. Names, folders, sizes, and
 * Spotlight only find candidates; a candidate becomes the asset's file
 * only when its full BLAKE3 hash equals the hash recorded at link time.
 *
 * Candidates, in order: the recorded path; files of the recorded size in a
 * bounded walk around the recorded folder (the folder, its parent and
 * grandparent, a few levels down) and under any extra roots (the project
 * file's folder, folders other linked media moved to); then files of that
 * size that Spotlight knows about. Same-name candidates are hashed first.
 */
import { execFile } from 'node:child_process';
import { readdir, stat } from 'node:fs/promises';
import path from 'node:path';

import type { MediaLink } from '../../shared/media-store';
import type { FileIdentity } from './hasher';

export type RelinkOptions = {
  hash: (file: string) => Promise<FileIdentity>;
  /** Spotlight lookup by exact size; injectable (tests pass none). */
  spotlight?: ((size: number, name: string) => Promise<string[]>) | null;
  /** Directories visited per search (default 4,000). */
  maxDirectories?: number;
  /** Candidates hashed per search (default 16). */
  maxHashed?: number;
  /** Levels below each root (default 4). */
  depth?: number;
  /** Folders never searched or chosen (the app's own data, its media store). */
  exclude?: readonly string[];
};

const inside = (file: string, folder: string) => {
  const relative = path.relative(folder, file);
  return relative === '' || (!!relative && !relative.startsWith('..') && !path.isAbsolute(relative));
};

/** Folders that never hold user media or are too large to walk. */
const SKIP = new Set(['node_modules', '.git', 'Library', '.Trash', '.Spotlight-V100', '.fseventsd', '.DocumentRevisions-V100', '.TemporaryItems']);

async function walk(root: string, size: number, depth: number, budget: { directories: number }, found: Set<string>, seen: Set<string>, exclude: readonly string[]): Promise<void> {
  const queue: Array<{ dir: string; level: number }> = [{ dir: root, level: 0 }];
  while (queue.length && budget.directories > 0) {
    const { dir, level } = queue.shift()!;
    if (seen.has(dir) || exclude.some((folder) => inside(dir, folder))) continue;
    seen.add(dir);
    budget.directories--;
    let entries;
    try { entries = await readdir(dir, { withFileTypes: true }); } catch { continue; }
    for (const entry of entries) {
      if (entry.name.startsWith('.')) continue;
      const full = path.join(dir, entry.name);
      if (entry.isDirectory()) {
        // Bundles (apps, Photos libraries, Final Cut libraries) stay closed.
        if (level < depth && !SKIP.has(entry.name) && !/\.(app|photoslibrary|fcpbundle|bundle|framework)$/i.test(entry.name)) queue.push({ dir: full, level: level + 1 });
      } else if (entry.isFile()) {
        try { if ((await stat(full)).size === size) found.add(full); } catch { /* vanished */ }
      }
    }
  }
}

/** `mdfind` for files of exactly `size` bytes (macOS). */
export function spotlightBySize(size: number): Promise<string[]> {
  if (process.platform !== 'darwin' || !Number.isSafeInteger(size) || size <= 0) return Promise.resolve([]);
  return new Promise((resolve) => {
    execFile('/usr/bin/mdfind', [`kMDItemFSSize == ${size}`], { timeout: 5000, maxBuffer: 4 * 1024 * 1024 }, (error, stdout) => {
      if (error && !stdout) { resolve([]); return; }
      resolve(String(stdout).split('\n').filter((line) => path.isAbsolute(line)).slice(0, 200));
    });
  });
}

async function nearestExisting(dir: string): Promise<string | null> {
  for (let current = dir; ; current = path.dirname(current)) {
    try { if ((await stat(current)).isDirectory()) return current; } catch { /* keep climbing */ }
    if (path.dirname(current) === current) return null;
  }
}

/** Finds the file whose content hash is `link.hash`, or null. */
export async function findByContent(link: MediaLink, searchRoots: readonly string[], options: RelinkOptions): Promise<{ link: MediaLink; identity: FileIdentity } | null> {
  const name = path.basename(link.path);
  const candidates = new Set<string>();
  try { if ((await stat(link.path)).size === link.size) candidates.add(link.path); } catch { /* moved */ }
  const budget = { directories: options.maxDirectories ?? 4000 };
  const depth = options.depth ?? 4;
  const seen = new Set<string>();
  const near = await nearestExisting(path.dirname(link.path));
  const roots: string[] = [];
  if (near) {
    roots.push(near);
    const parent = path.dirname(near);
    if (parent !== near && parent !== '/' && parent !== '/Users' && parent !== '/Volumes') roots.push(parent);
    const grandparent = path.dirname(parent);
    if (grandparent !== parent && grandparent !== '/' && grandparent !== '/Users' && grandparent !== '/Volumes') roots.push(grandparent);
  }
  for (const root of searchRoots) if (typeof root === 'string' && path.isAbsolute(root)) roots.push(root);
  const exclude = options.exclude ?? [];
  for (const root of roots) await walk(root, link.size, depth, budget, candidates, seen, exclude);
  if (options.spotlight !== null) {
    for (const file of await (options.spotlight ?? spotlightBySize)(link.size, name).catch(() => [] as string[])) {
      try { if ((await stat(file)).size === link.size) candidates.add(file); } catch { /* gone */ }
    }
  }
  const ordered = [...candidates].filter((file) => !exclude.some((folder) => inside(file, folder))).sort((a, b) => Number(path.basename(b) === name) - Number(path.basename(a) === name));
  let hashed = 0;
  for (const file of ordered) {
    if (hashed++ >= (options.maxHashed ?? 16)) break;
    try {
      const identity = await options.hash(file);
      if (identity.hash === link.hash && identity.size === link.size) {
        return { link: { path: file, hash: link.hash, size: link.size, mtimeMs: identity.mtimeMs, dev: identity.dev, ino: identity.ino }, identity };
      }
    } catch { /* unreadable candidate */ }
  }
  return null;
}
