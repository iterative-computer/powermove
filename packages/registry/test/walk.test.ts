import { describe, expect, test } from 'bun:test';
import { mkdtemp, mkdir, writeFile, symlink, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { snapshotDir, walkDir } from '../src/node';

describe('node walker', () => {
  test('rejects symlinks', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'registry-walk-'));
    try {
      await writeFile(join(dir, 'real'), 'ok');
      await symlink('real', join(dir, 'link'));
      await expect(walkDir(dir)).rejects.toMatchObject({ code: 'symlink', path: 'link' });
    } finally { await rm(dir, { recursive: true, force: true }); }
  });

  test('does not descend excluded directories', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'registry-walk-'));
    try {
      await mkdir(join(dir, 'node_modules'));
      await symlink('missing', join(dir, 'node_modules', 'link'));
      await mkdir(join(dir, '.hidden'));
      await writeFile(join(dir, '.hidden', 'secret'), 'hidden');
      await writeFile(join(dir, 'visible'), 'ok');
      expect((await walkDir(dir)).map((entry) => entry.path)).toEqual(['visible']);
      expect((await snapshotDir(dir)).files.map((entry) => entry.path)).toEqual(['visible']);
    } finally { await rm(dir, { recursive: true, force: true }); }
  });
});
