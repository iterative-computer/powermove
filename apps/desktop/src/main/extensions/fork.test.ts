import * as fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { afterEach, describe, expect, it } from 'vitest';

import { forkBuiltinExtension } from './fork';

const temporaryDirectories: string[] = [];

async function temporaryDirectory(): Promise<string> {
  const directory = await fs.mkdtemp(path.join(os.tmpdir(), 'powermove-fork-test-'));
  temporaryDirectories.push(directory);
  return directory;
}

async function fixture(overrides: Record<string, unknown> = {}): Promise<{
  resourcesDir: string;
  targetDir: string;
  sourceDir: string;
}> {
  const root = await temporaryDirectory();
  const resourcesDir = path.join(root, 'builtins');
  const targetDir = path.join(root, 'user');
  const sourceDir = path.join(resourcesDir, 'timeline');
  await fs.mkdir(path.join(sourceDir, 'src'), { recursive: true });
  await fs.mkdir(targetDir);
  await fs.writeFile(path.join(sourceDir, 'manifest.json'), JSON.stringify({
    id: 'timeline',
    name: 'Timeline',
    version: '1.2.3',
    apiVersion: 1,
    entry: 'index.ts',
    contributes: ['panels'],
    customFutureField: { retained: true },
    ...overrides
  }, null, 2));
  await fs.writeFile(path.join(sourceDir, 'index.ts'), "import './src/panel';\n");
  await fs.writeFile(path.join(sourceDir, 'src', 'panel.ts'), 'export const panel = true;\n');
  return { resourcesDir, targetDir, sourceDir };
}

afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map((directory) =>
    fs.rm(directory, { recursive: true, force: true })));
});

describe('forkBuiltinExtension', () => {
  it('creates an editable fork and an untouched pristine merge base', async () => {
    const setup = await fixture({ replaces: ['toolbar'] });
    const originalManifest = await fs.readFile(path.join(setup.sourceDir, 'manifest.json'), 'utf8');

    const result = await forkBuiltinExtension({
      resourcesDir: setup.resourcesDir,
      id: 'timeline',
      targetDir: setup.targetDir,
      forkId: 'my-timeline'
    });

    expect(result).toEqual({
      forkId: 'my-timeline',
      dir: path.join(await fs.realpath(setup.targetDir), 'my-timeline'),
      files: ['index.ts', 'manifest.json', 'src/panel.ts'],
      forkedFrom: 'timeline@1.2.3'
    });
    const manifest = JSON.parse(await fs.readFile(path.join(result.dir, 'manifest.json'), 'utf8'));
    expect(manifest).toMatchObject({
      id: 'my-timeline',
      name: 'Timeline (fork)',
      version: '1.2.3',
      replaces: ['toolbar', 'timeline'],
      forkedFrom: 'timeline@1.2.3',
      author: 'user',
      customFutureField: { retained: true }
    });
    expect(await fs.readFile(path.join(result.dir, '.forked-from', 'manifest.json'), 'utf8'))
      .toBe(originalManifest);
    expect(await fs.readFile(path.join(result.dir, '.forked-from', 'src', 'panel.ts'), 'utf8'))
      .toBe('export const panel = true;\n');
  });

  it('uses the default fork id and preserves an explicitly provided author', async () => {
    const setup = await fixture({ author: 'agent' });

    const result = await forkBuiltinExtension({
      resourcesDir: setup.resourcesDir,
      id: 'timeline',
      targetDir: setup.targetDir
    });

    expect(result.forkId).toBe('timeline-fork');
    await expect(fs.readFile(path.join(result.dir, 'manifest.json'), 'utf8')).resolves.toContain('"author": "agent"');
  });

  it('omits tests, test directories, Finder metadata, and hidden directories from both trees', async () => {
    const setup = await fixture();
    await fs.mkdir(path.join(setup.sourceDir, '__tests__'));
    await fs.mkdir(path.join(setup.sourceDir, 'src', '__tests__'));
    await fs.mkdir(path.join(setup.sourceDir, '.cache'));
    await fs.writeFile(path.join(setup.sourceDir, 'index.test.ts'), 'no');
    await fs.writeFile(path.join(setup.sourceDir, 'src', 'panel.test.ts'), 'no');
    await fs.writeFile(path.join(setup.sourceDir, '__tests__', 'index.ts'), 'no');
    await fs.writeFile(path.join(setup.sourceDir, 'src', '__tests__', 'nested.ts'), 'no');
    await fs.writeFile(path.join(setup.sourceDir, '.cache', 'data.ts'), 'no');
    await fs.writeFile(path.join(setup.sourceDir, '.DS_Store'), 'no');

    const result = await forkBuiltinExtension({
      resourcesDir: setup.resourcesDir,
      targetDir: setup.targetDir,
      id: 'timeline'
    });

    expect(result.files).toEqual(['index.ts', 'manifest.json', 'src/panel.ts']);
    expect(await fs.readdir(result.dir)).toEqual(['.forked-from', 'index.ts', 'manifest.json', 'src']);
    expect(await fs.readdir(path.join(result.dir, '.forked-from'))).toEqual(['index.ts', 'manifest.json', 'src']);
  });

  it('rejects invalid ids, missing built-ins, existing targets, and source symlinks without partial output', async () => {
    const setup = await fixture();
    await expect(forkBuiltinExtension({ ...setup, id: '../timeline' })).rejects.toThrow('Invalid built-in');
    await expect(forkBuiltinExtension({ ...setup, id: 'missing-ext' })).rejects.toThrow('does not exist');
    await fs.mkdir(path.join(setup.targetDir, 'timeline-fork'));
    await expect(forkBuiltinExtension({ ...setup, id: 'timeline' })).rejects.toThrow('already exists');

    await fs.rm(path.join(setup.targetDir, 'timeline-fork'), { recursive: true });
    const outside = path.join(path.dirname(setup.resourcesDir), 'outside.ts');
    await fs.writeFile(outside, 'secret');
    await fs.symlink(outside, path.join(setup.sourceDir, 'linked.ts'));
    await expect(forkBuiltinExtension({ ...setup, id: 'timeline' })).rejects.toThrow('symbolic link');
    await expect(fs.stat(path.join(setup.targetDir, 'timeline-fork'))).rejects.toMatchObject({ code: 'ENOENT' });
  });
});
