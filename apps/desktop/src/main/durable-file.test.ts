import { chmod, mkdtemp, readFile, rename, rm, stat, symlink, unlink, utimes, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { checkVersion, fileVersion, publishFile } from './durable-file';

let directory: string, destination: string, temporary: string;
const newHash = createHash('sha256').update('new save').digest('hex');
beforeEach(async () => {
  directory = await mkdtemp(path.join(tmpdir(), 'pm-file-version-'));
  destination = path.join(directory, 'Project.pmv'); temporary = path.join(directory, 'staged.tmp');
  await writeFile(destination, 'original'); await writeFile(temporary, 'new save');
});
afterEach(async () => { await rm(directory, { recursive: true, force: true }); });

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
import { createHash } from 'node:crypto';
