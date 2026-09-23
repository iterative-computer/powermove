import { describe, expect, test } from 'bun:test';
import { mkdtemp, mkdir, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, dirname } from 'node:path';
import { REGISTRY_LIMITS, type RegistryLimits } from '../src/limits';
import { isExcludedPath, normalizePath, snapshot, type SnapshotInput } from '../src/snapshot';

const enc = new TextEncoder();
const input = (path: string, text = ''): SnapshotInput => ({ path, bytes: enc.encode(text) });
const code = async (files: SnapshotInput[], expected: string, limits: RegistryLimits = REGISTRY_LIMITS) => {
  await expect(snapshot(files, limits)).rejects.toMatchObject({ code: expected });
};

describe('snapshot', () => {
  test('exclusions and empty tree', async () => {
    const base = [input('src/main.ts', 'code')];
    const extras = [input('.env', 'secret'), input('src/.cache/x', 'junk'), input('node_modules/a', 'pkg')];
    expect(isExcludedPath('x/.hidden/y')).toBe(true);
    expect(isExcludedPath('src/node_modules/x')).toBe(true);
    expect((await snapshot([...base, ...extras])).treeSha).toBe((await snapshot(base)).treeSha);
    await code(extras, 'empty_tree');
  });

  test('normalization and invalid paths', async () => {
    expect(normalizePath('cafe\u0301.txt')).toBe('café.txt');
    for (const path of ['', '/abs', 'a//b', 'a/./b', 'a/../b', 'a\\b', 'a\0b']) await code([input(path)], 'bad_path');
    await code([input('é'.repeat(101))], 'path_too_long');
    await code([input('café'), input('cafe\u0301')], 'duplicate_path');
  });

  test('collisions', async () => {
    await code([input('Alpha'), input('alpha')], 'case_collision');
    await code([input('a'), input('a/b')], 'file_dir_collision');
    await code([input('a/b'), input('a')], 'file_dir_collision');
  });

  test('limits and deduplicated objects', async () => {
    await code([input('a'), input('b')], 'too_many_files', { ...REGISTRY_LIMITS, files: 1 });
    await code([input('a', 'abc')], 'file_too_large', { ...REGISTRY_LIMITS, fileBytes: 2 });
    await code([input('a', 'ab'), input('b', 'cd')], 'tree_too_large', { ...REGISTRY_LIMITS, treeBytes: 3 });
    const result = await snapshot([input('b', 'same'), input('a', 'same')]);
    expect(result.files.map((file) => file.path)).toEqual(['a', 'b']);
    expect(result.objects.filter((object) => object.type === 'blob')).toHaveLength(1);
    expect(result.totalBytes).toBe(8);
  });

  test.skipIf(!Bun.which('git'))('matches real Git tree and blob identities', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'registry-git-'));
    const files = [
      input('a', 'a'), input('a.b/inner', 'inner'), input('a-b', 'dash'),
      input('nested/deep/file.ts', 'hello'), input('café.txt', 'NFC'),
      input('日本語/✨.txt', 'unicode'), input('zero', ''),
    ];
    const git = async (...args: string[]) => {
      const process = Bun.spawn(['git', '-c', 'core.excludesFile=/dev/null', '-c', 'core.autocrlf=false', '-c', 'core.precomposeunicode=true', ...args], { cwd: dir, stdout: 'pipe', stderr: 'pipe' });
      const [stdout, stderr, status] = await Promise.all([new Response(process.stdout).text(), new Response(process.stderr).text(), process.exited]);
      if (status !== 0) throw new Error(stderr);
      return stdout.trim();
    };
    try {
      for (const file of files) { const path = join(dir, file.path); await mkdir(dirname(path), { recursive: true }); await writeFile(path, file.bytes); }
      await git('init', '-q');
      await git('add', '-A');
      const result = await snapshot(files);
      expect(result.treeSha).toBe(await git('write-tree'));
      for (const file of result.files) expect(file.sha).toBe(await git('hash-object', file.path));
      expect((await snapshot([...files, input('.secret'), input('node_modules/pkg')])).treeSha).toBe(result.treeSha);
    } finally { await rm(dir, { recursive: true, force: true }); }
  });
});
