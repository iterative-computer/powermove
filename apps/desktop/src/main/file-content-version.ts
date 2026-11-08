import { createHash } from 'node:crypto';
import { constants, type Stats } from 'node:fs';
import { lstat, open } from 'node:fs/promises';
import { sameFile } from './durable-file';
import { readWithRetry } from './read-retry';

/** Metadata is a cache hint, never proof that an unfamiliar file has our bytes.
 * Include ctime so edits that restore mtime still invalidate the cached hash. */
export function contentIdentity(info: Stats): string {
  return `${info.dev}:${info.ino}:${info.size}:${info.mtimeMs}:${info.ctimeMs}`;
}

/** Require one stable full hash. A provider's metadata update or atomic file
 * replacement during a read gets a fresh attempt, including a fresh checksum.
 * Never accept a changed inode/timestamp just because a previous hash matched. */
export async function fileContentVersion(file: string): Promise<{ info: Stats; sha256: string }> {
  for (let attempt = 0; attempt < 3; attempt++) {
    const handle = await readWithRetry(() => open(file, constants.O_RDONLY | constants.O_NOFOLLOW));
    try {
      const before = await handle.stat();
      if (!before.isFile()) throw new Error('This is not a readable file.');
      const hash = createHash('sha256');
      const buffer = Buffer.allocUnsafe(Math.min(1024 * 1024, before.size));
      for (let offset = 0; offset < before.size;) {
        const { bytesRead } = await readWithRetry(() => handle.read(buffer, 0, Math.min(buffer.length, before.size - offset), offset));
        if (!bytesRead) throw new Error('This file is not fully available. Download it in Finder, then try again.');
        hash.update(buffer.subarray(0, bytesRead));
        offset += bytesRead;
      }
      const sha256 = hash.digest('hex');
      const after = await handle.stat();
      const current = await lstat(file);
      if (sameFile(before, after) && sameFile(after, current)) return { info: after, sha256 };
    } finally { await handle.close(); }
  }
  throw new Error('This file changed while it was being read. Try again when syncing has finished.');
}

/** Bounded cache for expensive media checksums. A metadata change always
 * re-hashes; identical bytes still produce the same downstream cache key. */
export class FileContentCache {
  private entries = new Map<string, { identity: string; sha256: string }>();
  async hash(file: string): Promise<string> {
    const metadata = await lstat(file);
    if (!metadata.isFile()) throw new Error('This is not a readable file.');
    const known = this.entries.get(file);
    if (known?.identity === contentIdentity(metadata)) return known.sha256;
    const verified = await fileContentVersion(file);
    this.entries.delete(file);
    this.entries.set(file, { identity: contentIdentity(verified.info), sha256: verified.sha256 });
    if (this.entries.size > 128) this.entries.delete(this.entries.keys().next().value!);
    return verified.sha256;
  }
}
