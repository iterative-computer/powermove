import { readWithRetry } from './read-retry';
import { createHash, type Hash, randomUUID } from 'node:crypto';
import { constants, type Stats } from 'node:fs';
import { copyFile, lstat, open, rename, unlink, type FileHandle } from 'node:fs/promises';
import path from 'node:path';
type NativeFiles = typeof import('@powermove/macos-haptics');
let native: Promise<NativeFiles | null> | undefined;
// The standalone Node host does not ship the desktop's native addon. It uses
// the same transaction format with ordinary copies and fsync on those hosts.
function nativeFiles(): Promise<NativeFiles | null> {
  return native ??= process.platform === 'darwin' ? import('@powermove/macos-haptics').then(module =>
    (module as NativeFiles & { default?: NativeFiles }).default ?? module).catch((error: any) => {
      if (error.code === 'MODULE_NOT_FOUND' || error.code === 'ERR_MODULE_NOT_FOUND') return null;
      throw error;
    }) : Promise.resolve(null);
}
const swapFiles = async (source: string, destination: string) => (await nativeFiles())?.swapFiles(source, destination) ?? false;

export const temporarySibling = (destination: string) => path.join(path.dirname(destination), `.${path.basename(destination)}.${randomUUID()}.tmp`);
export async function syncFile(handle: FileHandle): Promise<void> {
  if (!await (await nativeFiles())?.fullSync(handle.fd)) await handle.sync();
}
export async function syncDirectory(directory: string): Promise<void> {
  const handle = await open(directory, 'r');
  try { await syncFile(handle); } finally { await handle.close(); }
}
export async function copySnapshot(source: string, destination: string): Promise<void> {
  if (!await (await nativeFiles())?.cloneFile(source, destination)) await copyFile(source, destination, constants.COPYFILE_EXCL | constants.COPYFILE_FICLONE);
}
export async function readExactly(handle: FileHandle, position: number, length: number): Promise<Buffer> {
  const data = Buffer.allocUnsafe(length);
  for (let offset = 0; offset < length;) {
    const { bytesRead } = await readWithRetry(() => handle.read(data, offset, length - offset, position + offset));
    if (!bytesRead) throw new Error('The project container is truncated.');
    offset += bytesRead;
  }
  return data;
}
export async function hashHandle(handle: FileHandle, offset = 0, length?: number): Promise<Hash> {
  const hash = createHash('sha256');
  if (length === 0) return hash;
  const buffer = Buffer.allocUnsafe(1024 * 1024);
  for (let position = offset; ;) {
    const count = length === undefined ? buffer.length : Math.min(buffer.length, offset + length - position);
    if (count <= 0) break;
    const { bytesRead } = await readWithRetry(() => handle.read(buffer, 0, count, position));
    if (!bytesRead) break;
    hash.update(buffer.subarray(0, bytesRead));
    position += bytesRead;
  }
  return hash;
}
export const sameIdentity = (a: Stats, b: Stats) => a.dev === b.dev && a.ino === b.ino;
const samePermissions = (a: Stats, b: Stats) => a.mode === b.mode && a.uid === b.uid && a.gid === b.gid;
export const sameFile = (a: Stats, b: Stats) => sameIdentity(a, b) && a.size === b.size && a.mtimeMs === b.mtimeMs && a.ctimeMs === b.ctimeMs;
export const changedFile = () => new Error('The project file was moved, deleted, or changed outside Powermove. Your edits are still open. Use Save As with a different name to keep both versions.');
export interface FileVersion { info: Stats; hash: string; digest: Hash }
export async function fileVersion(destination: string): Promise<FileVersion | undefined> {
  let info: Stats;
  try { info = await lstat(destination); } catch (error: any) { if (error.code === 'ENOENT') return; throw error; }
  // Never write through or replace a symlink supplied in a destination.
  if (!info.isFile()) throw new Error('The destination cannot be safely backed up. Choose another file with Save As.');
  const handle = await open(destination, 'r');
  try {
    if (!sameFile(info, await handle.stat())) throw changedFile();
    const digest = await hashHandle(handle);
    if (!sameFile(info, await handle.stat()) || !sameFile(info, await lstat(destination))) throw changedFile();
    return { info, hash: digest.copy().digest('hex'), digest };
  } finally { await handle.close(); }
}
export async function checkVersion(destination: string, expected?: FileVersion, verifyContent = false): Promise<void> {
  let current: Stats | undefined;
  try { current = await lstat(destination); } catch (error: any) { if (error.code !== 'ENOENT') throw error; }
  if (!expected) { if (current) throw changedFile(); return; }
  if (!current?.isFile() || !sameIdentity(expected.info, current) || !samePermissions(expected.info, current) || expected.info.size !== current.size) throw changedFile();
  // Timestamps also change for touches and filesystem metadata bookkeeping.
  // Accept those only with proof of identical bytes on the SAME inode. Never
  // use a timestamp tolerance, or let an equal hash excuse a replaced file.
  if (verifyContent || !sameFile(expected.info, current)) {
    const verified = await fileVersion(destination);
    if (!verified || !sameIdentity(expected.info, verified.info) || !samePermissions(expected.info, verified.info) || verified.hash !== expected.hash) throw changedFile();
  }
}

/** The displaced file belongs to another writer, including if renaming its
 * recovery copy fails. Callers must not unlink that path as staging cleanup. */
export class RecoveryFileError extends Error {
  constructor(message: string, readonly recoveryPath: string, cause?: unknown) {
    super(`${message} The other version was preserved at ${recoveryPath}`, { cause });
  }
}
async function preserveConflict(temporary: string, destination: string, message: string): Promise<never> {
  let preserved = temporary;
  try {
    const recovery = `${temporary}.conflict.pmv`;
    await rename(temporary, recovery); preserved = recovery;
    await syncDirectory(path.dirname(destination));
  } catch (error) {
    throw new RecoveryFileError(`${message} Recovery finalization failed.`, preserved, error);
  }
  throw new RecoveryFileError(message, preserved);
}

/** Publish only a flushed sibling. On macOS exchange preserves the displaced
 * inode until its content has been checked, closing the long upload race. */
export async function publishFile(temporary: string, destination: string, expected: FileVersion | undefined, newHash: string,
  onPublished: () => void = () => undefined): Promise<void> {
  // Exchange verifies the displaced bytes; the fallback hashes immediately
  // before rename. Neither trusts timestamps alone at publication.
  await checkVersion(destination, expected);
  let published = false;
  const committed = () => { published = true; onPublished(); };
  try {
    const incomingInfo = await lstat(temporary);
    if (expected && await swapFiles(temporary, destination)) {
      let displaced: FileVersion | undefined;
      try { displaced = await fileVersion(temporary); }
      catch {
        await preserveConflict(temporary, destination, 'Could not verify the displaced project.');
      }
      if (!displaced || displaced.hash !== expected.hash || !sameIdentity(displaced.info, expected.info) || !samePermissions(displaced.info, expected.info)) {
        // Only roll back our own published bytes. If another writer has already
        // edited them, keep both versions and report the recovery file's path.
        const current = await fileVersion(destination).catch(() => undefined);
        if (current?.hash === newHash && sameIdentity(current.info, incomingInfo) && await swapFiles(temporary, destination).catch(() => false)) {
          // The check and rollback exchange are separate operations. A writer
          // can edit/replace our file between them; never let dispose() discard
          // that newly displaced work, even though the original is restored.
          const rolledBack = await fileVersion(temporary).catch(() => undefined);
          if (!rolledBack || rolledBack.hash !== newHash || !sameIdentity(rolledBack.info, incomingInfo)) {
            await preserveConflict(temporary, destination, 'The project changed during saving. The original was restored.');
          }
          await syncDirectory(path.dirname(destination));
          throw changedFile();
        }
        await preserveConflict(temporary, destination, 'The project changed during saving.');
      }
      committed();
      await unlink(temporary);
    } else if (!expected) {
      // macOS exclusive rename also supports volumes without hard links. A file
      // appearing after the check cannot be overwritten by first Save/Save As.
      if (await (await nativeFiles())?.installFile(temporary, destination)) committed();
      else {
        const { link } = await import('node:fs/promises');
        await link(temporary, destination);
        committed();
        await unlink(temporary);
      }
    } else {
      await checkVersion(destination, expected, true);
      await rename(temporary, destination);
      committed();
    }
    await syncDirectory(path.dirname(destination));
  } catch (error) {
    if (published) throw new Error('The project file was written, but save finalization failed. Your edits are still open. Try Save again to confirm, or Save As to keep a separate copy. ' + (error instanceof Error ? error.message : String(error)), { cause: error });
    throw error;
  }
}
