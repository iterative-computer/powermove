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
  // Node cannot open directory handles for fsync on Windows. File handles
  // are still flushed before publication; do not turn a successful save into
  // an EISDIR/EPERM failure after replacing the destination.
  if (process.platform === 'win32') return;
  const handle = await open(directory, 'r');
  try { await syncFile(handle); } finally { await handle.close(); }
}
export async function copySnapshot(source: string, destination: string): Promise<void> {
  if (!await (await nativeFiles())?.cloneFile(source, destination)) await copyFile(source, destination, constants.COPYFILE_EXCL | constants.COPYFILE_FICLONE);
}
export async function readExactly(handle: FileHandle, position: number, length: number): Promise<Buffer> {
  const data = Buffer.allocUnsafe(length);
  for (let offset = 0; offset < length;) {
    const { bytesRead } = await handle.read(data, offset, length - offset, position + offset);
    if (!bytesRead) throw new Error('The project container is truncated.');
    offset += bytesRead;
  }
  return data;
}
export async function hashHandle(handle: FileHandle, offset = 0, length?: number): Promise<Hash> {
  const hash = createHash('sha256');
  if (length === 0) return hash;
  for await (const data of handle.createReadStream({ start: offset, ...(length === undefined ? {} : { end: offset + length - 1 }), autoClose: false, highWaterMark: 1024 * 1024 })) hash.update(data);
  return hash;
}
export const sameFile = (a: Stats, b: Stats) => a.dev === b.dev && a.ino === b.ino && a.size === b.size && a.mtimeMs === b.mtimeMs && a.ctimeMs === b.ctimeMs;
export const changedFile = () => new Error('The project file was moved, deleted, or changed outside Powermove. Use Save As to avoid overwriting other work.');
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
export async function checkVersion(destination: string, expected?: FileVersion): Promise<void> {
  let current: Stats | undefined;
  try { current = await lstat(destination); } catch (error: any) { if (error.code !== 'ENOENT') throw error; }
  if (expected ? !current || !sameFile(expected.info, current) : !!current) throw changedFile();
}

/** Publish only a flushed sibling. On macOS exchange preserves the displaced
 * inode until its content has been checked, closing the long upload race. */
export async function publishFile(temporary: string, destination: string, expected: FileVersion | undefined, newHash: string): Promise<void> {
  await checkVersion(destination, expected);
  if (expected && await swapFiles(temporary, destination)) {
    let displaced: FileVersion | undefined;
    try { displaced = await fileVersion(temporary); }
    catch {
      const preserved = `${temporary}.conflict.pmv`;
      await rename(temporary, preserved);
      await syncDirectory(path.dirname(destination));
      throw new Error(`Could not verify the displaced project. It was preserved at ${preserved}`);
    }
    if (!displaced || displaced.hash !== expected.hash || displaced.info.ino !== expected.info.ino) {
      // Only roll back our own published bytes. If another writer has already
      // edited them, keep both versions and report the recovery file's path.
      const current = await fileVersion(destination).catch(() => undefined);
      if (current?.hash === newHash && await swapFiles(temporary, destination).catch(() => false)) {
        await syncDirectory(path.dirname(destination));
        throw changedFile();
      }
      const preserved = `${temporary}.conflict.pmv`;
      await rename(temporary, preserved);
      await syncDirectory(path.dirname(destination));
      throw new Error(`The project changed during saving. Its other version was preserved at ${preserved}`);
    }
    await unlink(temporary);
  } else if (!expected) {
    // macOS exclusive rename also supports volumes without hard links. A file
    // appearing after the check cannot be overwritten by first Save/Save As.
    if (!await (await nativeFiles())?.installFile(temporary, destination)) {
      const { link } = await import('node:fs/promises');
      await link(temporary, destination);
      await unlink(temporary);
    }
  } else {
    await checkVersion(destination, expected);
    await rename(temporary, destination);
  }
  await syncDirectory(path.dirname(destination));
}
