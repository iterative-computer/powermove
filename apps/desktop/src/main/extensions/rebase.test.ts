import {
  access,
  mkdir,
  mkdtemp,
  readFile,
  readdir,
  rm,
  symlink,
  writeFile
} from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { afterEach, describe, expect, it } from 'vitest';

import { prepareExtensionStage, publishExtensionChanges } from '../codex/change-history';
import { readForkRebaseInfo, stageForkRebase } from './rebase';

const temporaryDirectories: string[] = [];

afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map((directory) =>
    rm(directory, { recursive: true, force: true })));
});

async function temporaryDirectory(): Promise<string> {
  const directory = await mkdtemp(path.join(os.tmpdir(), 'powermove-rebase-'));
  temporaryDirectories.push(directory);
  return directory;
}

async function writeTree(root: string, files: Record<string, string>): Promise<void> {
  for (const [relative, contents] of Object.entries(files)) {
    const target = path.join(root, relative);
    await mkdir(path.dirname(target), { recursive: true });
    await writeFile(target, contents);
  }
}

function builtinManifest(id: string, version: string): string {
  return JSON.stringify({
    id,
    name: id,
    version,
    apiVersion: 1,
    entry: 'index.ts',
    author: 'powermove'
  });
}

function forkManifest(
  forkId: string,
  builtinId: string,
  baseVersion: string,
  overrides: Record<string, unknown> = {}
): string {
  return JSON.stringify({
    id: forkId,
    name: forkId,
    version: '1.0.0',
    apiVersion: 1,
    entry: 'index.ts',
    author: 'user',
    replaces: [builtinId],
    forkedFrom: `${builtinId}@${baseVersion}`,
    ...overrides
  });
}

async function fixture(): Promise<{
  root: string;
  userExtensionsDir: string;
  builtinExtensionsDir: string;
  stagingDirectory: string;
}> {
  const root = await temporaryDirectory();
  const userExtensionsDir = path.join(root, 'user');
  const builtinExtensionsDir = path.join(root, 'builtins');
  const stagingDirectory = path.join(root, 'stage');
  const fork = path.join(userExtensionsDir, 'custom-timeline');
  const oldBase = path.join(fork, '.forked-from');
  const current = path.join(builtinExtensionsDir, 'timeline');

  await writeTree(oldBase, {
    'manifest.json': builtinManifest('timeline', '1.0.0'),
    'index.ts': 'export const value = "old";\n',
    'shared.ts': 'export const shared = "old";\n',
    'removed-by-user.ts': 'export const removed = true;\n',
    'removed-upstream.ts': 'export const upstreamRemoved = true;\n'
  });
  await writeTree(fork, {
    'manifest.json': forkManifest('custom-timeline', 'timeline', '1.0.0'),
    'index.ts': 'export const value = "user";\n',
    'shared.ts': 'export const shared = "user";\n',
    'removed-upstream.ts': 'export const upstreamRemoved = true;\n',
    'user-added.ts': 'export const added = true;\n',
    '.settings': 'preserve me\n'
  });
  await writeTree(current, {
    'manifest.json': builtinManifest('timeline', '2.0.0'),
    'index.ts': 'export const value = "old";\n',
    'shared.ts': 'export const shared = "upstream";\n',
    'removed-by-user.ts': 'export const removed = true;\n',
    'upstream-added.ts': 'export const upstreamAdded = true;\n'
  });
  return { root, userExtensionsDir, builtinExtensionsDir, stagingDirectory };
}

describe('fork rebase staging', () => {
  it('creates a contained three-way layout, preserves user files, and installs the new hidden base', async () => {
    const setup = await fixture();
    const result = await stageForkRebase({
      forkId: 'custom-timeline',
      stagingDirectory: setup.stagingDirectory,
      userExtensionsDir: setup.userExtensionsDir,
      builtinExtensionsDir: setup.builtinExtensionsDir
    });

    expect(Object.keys(result)).toEqual([
      'forkId', 'workingDir', 'baseDir', 'oursDir',
      'changedByUser', 'changedUpstream', 'conflicts'
    ]);
    expect(result).toMatchObject({
      forkId: 'custom-timeline',
      workingDir: path.join(setup.stagingDirectory, 'custom-timeline'),
      baseDir: path.join(setup.stagingDirectory, '.rebase', 'custom-timeline', 'base'),
      oursDir: path.join(setup.stagingDirectory, '.rebase', 'custom-timeline', 'ours')
    });
    for (const directory of [result.workingDir, result.baseDir, result.oursDir]) {
      expect(path.relative(setup.stagingDirectory, directory)).not.toMatch(/^\.\.(?:\/|$)/);
    }

    expect(await readFile(path.join(result.workingDir, '.settings'), 'utf8')).toBe('preserve me\n');
    expect(await readFile(path.join(result.workingDir, 'user-added.ts'), 'utf8')).toContain('added');
    expect(await readFile(path.join(result.baseDir, 'shared.ts'), 'utf8')).toContain('old');
    expect(await readFile(path.join(result.oursDir, 'shared.ts'), 'utf8')).toContain('upstream');
    expect(await readFile(path.join(result.workingDir, '.forked-from', 'shared.ts'), 'utf8'))
      .toContain('upstream');
    const workingManifest = JSON.parse(await readFile(path.join(result.workingDir, 'manifest.json'), 'utf8'));
    expect(workingManifest).toMatchObject({
      id: 'custom-timeline',
      version: '1.0.0',
      replaces: ['timeline'],
      forkedFrom: 'timeline@2.0.0'
    });
  });

  it('reports sorted content additions, deletions, edits, and their intersection as conflicts', async () => {
    const setup = await fixture();
    const result = await stageForkRebase({
      forkId: 'custom-timeline',
      stagingDirectory: setup.stagingDirectory,
      userExtensionsDir: setup.userExtensionsDir,
      builtinExtensionsDir: setup.builtinExtensionsDir
    });

    expect(result.changedByUser).toEqual([
      '.settings',
      'index.ts',
      'manifest.json',
      'removed-by-user.ts',
      'shared.ts',
      'user-added.ts'
    ]);
    expect(result.changedUpstream).toEqual([
      'manifest.json',
      'removed-upstream.ts',
      'shared.ts',
      'upstream-added.ts'
    ]);
    expect(result.conflicts).toEqual(['manifest.json', 'shared.ts']);
  });

  it('reads validated fork update metadata without staging it', async () => {
    const setup = await fixture();
    await expect(readForkRebaseInfo({
      forkId: 'custom-timeline',
      userExtensionsDir: setup.userExtensionsDir,
      builtinExtensionsDir: setup.builtinExtensionsDir
    })).resolves.toEqual({
      forkId: 'custom-timeline',
      forkedFrom: 'timeline',
      base: '1.0.0',
      current: '2.0.0'
    });
    await expect(access(setup.stagingDirectory)).rejects.toMatchObject({ code: 'ENOENT' });
  });

  it('rejects unknown, non-user, non-fork, and inconsistent fork metadata', async () => {
    const setup = await fixture();
    const options = {
      stagingDirectory: setup.stagingDirectory,
      userExtensionsDir: setup.userExtensionsDir,
      builtinExtensionsDir: setup.builtinExtensionsDir
    };

    await expect(stageForkRebase({ ...options, forkId: 'missing-fork' }))
      .rejects.toThrow(/Unknown extension/);
    await expect(stageForkRebase({ ...options, forkId: 'timeline' }))
      .rejects.toThrow(/Only user extensions/);

    const manifestPath = path.join(setup.userExtensionsDir, 'custom-timeline', 'manifest.json');
    await writeFile(manifestPath, forkManifest('custom-timeline', 'timeline', '1.0.0', { forkedFrom: undefined }));
    await expect(stageForkRebase({ ...options, forkId: 'custom-timeline' }))
      .rejects.toThrow(/not a fork/);

    await writeFile(manifestPath, forkManifest('custom-timeline', 'timeline', '1.0.0', { forkedFrom: 'timeline' }));
    await expect(stageForkRebase({ ...options, forkId: 'custom-timeline' }))
      .rejects.toThrow(/Malformed forkedFrom/);

    await writeFile(manifestPath, forkManifest('custom-timeline', 'timeline', '1.0.0', { replaces: [] }));
    await expect(stageForkRebase({ ...options, forkId: 'custom-timeline' }))
      .rejects.toThrow(/must replace timeline/);

    await writeFile(manifestPath, forkManifest('custom-timeline', 'timeline', '9.0.0'));
    await expect(stageForkRebase({ ...options, forkId: 'custom-timeline' }))
      .rejects.toThrow(/base is version 1\.0\.0/);

    await writeFile(manifestPath, forkManifest('custom-timeline', 'timeline', '1.0.0'));
    await writeFile(
      path.join(setup.builtinExtensionsDir, 'timeline', 'manifest.json'),
      builtinManifest('timeline', '1.0.0')
    );
    await expect(stageForkRebase({ ...options, forkId: 'custom-timeline' }))
      .rejects.toThrow(/already based on the shipped timeline@1\.0\.0/);
  });

  it('rejects missing bases, malformed ids, unsafe roots, and symbolic links', async () => {
    const setup = await fixture();
    const options = {
      stagingDirectory: setup.stagingDirectory,
      userExtensionsDir: setup.userExtensionsDir,
      builtinExtensionsDir: setup.builtinExtensionsDir
    };

    await expect(stageForkRebase({ ...options, forkId: '../timeline' }))
      .rejects.toThrow(/Invalid extension id/);
    await expect(stageForkRebase({ ...options, forkId: 'custom-timeline', stagingDirectory: 'relative' }))
      .rejects.toThrow(/absolute/);
    await expect(stageForkRebase({
      ...options,
      forkId: 'custom-timeline',
      stagingDirectory: path.join(setup.userExtensionsDir, 'nested-stage')
    })).rejects.toThrow(/must not overlap/);

    await rm(path.join(setup.userExtensionsDir, 'custom-timeline', '.forked-from'), { recursive: true });
    await expect(stageForkRebase({ ...options, forkId: 'custom-timeline' }))
      .rejects.toThrow(/missing its \.forked-from/);

    const restored = await fixture();
    await symlink(
      path.join(restored.root, 'outside.ts'),
      path.join(restored.userExtensionsDir, 'custom-timeline', 'linked.ts')
    );
    await expect(stageForkRebase({
      forkId: 'custom-timeline',
      stagingDirectory: restored.stagingDirectory,
      userExtensionsDir: restored.userExtensionsDir,
      builtinExtensionsDir: restored.builtinExtensionsDir
    })).rejects.toThrow(/symbolic links/);
  });

  it('excludes the top-level staging .rebase metadata directory from extension promotion', async () => {
    const setup = await fixture();
    const historyRoot = path.join(setup.root, 'history');
    const prepared = await prepareExtensionStage({
      liveDirectory: setup.userExtensionsDir,
      stagingDirectory: setup.stagingDirectory,
      historyRoot,
      projectId: 'project-1',
      runId: 'rebase-run'
    });
    await stageForkRebase({
      forkId: 'custom-timeline',
      stagingDirectory: setup.stagingDirectory,
      userExtensionsDir: setup.userExtensionsDir,
      builtinExtensionsDir: setup.builtinExtensionsDir
    });

    await expect(publishExtensionChanges(prepared, [{
      id: 'custom-timeline',
      action: 'updated',
      summary: 'Rebased onto timeline 2.0.0'
    }])).resolves.toMatchObject({ id: 'rebase-run' });
    expect(JSON.parse(await readFile(
      path.join(setup.userExtensionsDir, 'custom-timeline', 'manifest.json'),
      'utf8'
    ))).toMatchObject({ forkedFrom: 'timeline@2.0.0' });
    expect(await readdir(setup.userExtensionsDir)).toEqual(['custom-timeline']);
    await expect(access(path.join(setup.userExtensionsDir, '.rebase')))
      .rejects.toMatchObject({ code: 'ENOENT' });
  });
});
