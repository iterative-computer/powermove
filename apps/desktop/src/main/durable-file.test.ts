import { chmod, mkdtemp, open, readFile, rename, rm, stat, symlink, unlink, utimes, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { checkVersion, fileVersion, publishFile, verifyOpenFile } from './durable-file';

// Runs a side effect between the reads that hash a file.
const reads = vi.hoisted(() => ({ before: [] as Array<() => Promise<void>> }));
vi.mock('./read-retry', async importOriginal => {
  const actual = await importOriginal<typeof import('./read-retry')>();
  return { ...actual, readWithRetry: async <T>(read: () => Promise<T>) => { await reads.before.shift()?.(); return actual.readWithRetry(read); } };
});
const touch = (file: string) => async () => { await chmod(file, (await stat(file)).mode & 0o777); };

let directory: string, destination: string, temporary: string;
const newHash = createHash('sha256').update('new save').digest('hex');
beforeEach(async () => {
  directory = await mkdtemp(path.join(tmpdir(), 'pm-file-version-'));
  destination = path.join(directory, 'Project.pmv'); temporary = path.join(directory, 'staged.tmp');
  await writeFile(destination, 'original'); await writeFile(temporary, 'new save');
});
afterEach(async () => { reads.before = []; await rm(directory, { recursive: true, force: true }); });

describe('file version protection', () => {
  it('verifies identical content after a timestamp-only change on the original inode', async () => {
    const expected = await fileVersion(destination);
    await utimes(destination, new Date(1000), new Date(2000));
    await expect(checkVersion(destination, expected)).resolves.toBeUndefined();
  });
  it('rejects a same-size external edit even when its mtime is restored', async () => {
    const expected = (await fileVersion(destination))!;
    await writeFile(destination, 'external');
    await utimes(destination, expected.info.atime, expected.info.mtime);
    await expect(publishFile(temporary, destination, expected, newHash)).rejects.toThrow('changed');
    expect(await readFile(destination, 'utf8')).toBe('external');
  });
  it('checks content at publication when metadata cannot distinguish different bytes', async () => {
    const expected = (await fileVersion(destination))!;
    await writeFile(destination, 'external');
    // Model a coarse-timestamp filesystem reporting the same stat for both
    // revisions without mocking the actual byte reads or publication.
    expected.info = await stat(destination);
    await expect(publishFile(temporary, destination, expected, newHash)).rejects.toThrow('changed');
    expect(await readFile(destination, 'utf8')).toBe('external');
  });
  it.skipIf(process.platform === 'win32')('never overwrites changed file permissions just because the bytes match', async () => {
    const expected = await fileVersion(destination);
    await chmod(destination, 0o400);
    await expect(publishFile(temporary, destination, expected, newHash)).rejects.toThrow('changed');
    expect((await stat(destination)).mode & 0o777).toBe(0o400);
    expect(await readFile(destination, 'utf8')).toBe('original');
  });
  it('does not let identical bytes excuse a different inode at publication', async () => {
    const expected = (await fileVersion(destination))!;
    const replacement = path.join(directory, 'replacement');
    await writeFile(replacement, 'original'); await rename(replacement, destination);
    await expect(publishFile(temporary, destination, expected, newHash)).rejects.toThrow('changed');
    expect(await readFile(destination, 'utf8')).toBe('original');
  });
  it.each(['move', 'delete', 'symlink'])('preserves %s detection throughout a transaction', async operation => {
    const expected = await fileVersion(destination), moved = path.join(directory, 'moved.pmv');
    if (operation === 'delete') await unlink(destination);
    else {
      await rename(destination, moved);
      if (operation === 'symlink') await symlink(moved, destination);
    }
    await expect(publishFile(temporary, destination, expected, newHash)).rejects.toThrow('changed');
    if (operation !== 'delete') expect(await readFile(moved, 'utf8')).toBe('original');
    expect(await readFile(temporary, 'utf8')).toBe('new save');
  });
});

describe('cloud metadata churn', () => {
  it('re-reads a file whose metadata alone changes while it is being hashed', async () => {
    reads.before = [touch(destination)];
    const version = await fileVersion(destination);
    expect(version?.hash).toBe(createHash('sha256').update('original').digest('hex'));
  });
  it('accepts identical bytes when metadata keeps changing on every read', async () => {
    reads.before = Array.from({ length: 20 }, () => touch(destination));
    const version = await fileVersion(destination);
    expect(version?.hash).toBe(createHash('sha256').update('original').digest('hex'));
  });
  it('never accepts content written while the file is being hashed', async () => {
    reads.before = Array.from({ length: 20 }, (_, index) => async () => {
      // Same size, different bytes each time.
      await writeFile(destination, `origin${String(index).padStart(2, '0')}`);
    });
    await expect(fileVersion(destination)).rejects.toThrow('changed');
  });
  it('waits out a cloud provider briefly refusing reads', async () => {
    reads.before = Array.from({ length: 4 }, () => async () => { throw Object.assign(new Error('Resource deadlock avoided'), { code: 'EDEADLK', errno: -11 }); });
    const version = await fileVersion(destination);
    expect(version?.hash).toBe(createHash('sha256').update('original').digest('hex'));
  });
  it('verifies an open handle by content when only its metadata changed', async () => {
    const expected = (await fileVersion(destination))!;
    const handle = await open(destination, 'r');
    try {
      await touch(destination)();
      await expect(verifyOpenFile(handle, expected)).resolves.toBeUndefined();
      await writeFile(destination, 'external');
      await utimes(destination, expected.info.atime, expected.info.mtime);
      await expect(verifyOpenFile(handle, expected)).rejects.toThrow('changed');
    } finally { await handle.close(); }
  });
});
import { createHash } from 'node:crypto';
