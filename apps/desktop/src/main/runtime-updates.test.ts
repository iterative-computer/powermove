import { mkdir, mkdtemp, readdir, rm, symlink, utimes, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { isNewerVersion, sweepRuntimeScratch } from './runtime-updates';

it('only treats strictly higher versions as newer', () => {
  expect(isNewerVersion('2.1.284', '2.1.280')).toBe(true);
  expect(isNewerVersion('2.2.0', '2.1.999')).toBe(true);
  expect(isNewerVersion('2.1.280', '2.1.280')).toBe(false);
  expect(isNewerVersion('2.1.279', '2.1.280')).toBe(false);
});

describe('sweepRuntimeScratch', () => {
  let tmpDir: string;
  const now = Date.now();
  const hour = 60 * 60 * 1000;

  beforeEach(async () => { tmpDir = await mkdtemp(path.join(os.tmpdir(), 'runtime-sweep-test-')); });
  afterEach(async () => { await rm(tmpDir, { recursive: true, force: true }); });

  async function fixture(name: string, marker: 'archive' | 'package' | null, age = 2 * hour): Promise<string> {
    const dir = path.join(tmpDir, name);
    await mkdir(dir);
    if (marker === 'archive') await writeFile(path.join(dir, 'package.tgz'), 'partial download');
    if (marker === 'package') await mkdir(path.join(dir, 'package'));
    const modified = new Date(now - age);
    await utimes(dir, modified, modified);
    return dir;
  }

  it('removes only stale provider scratch directories with an archive or extracted package', async () => {
    await fixture('powermove-claude-abandoned', 'archive');
    await fixture('powermove-codex-abandoned', 'package');
    await fixture('powermove-claude-fresh', 'archive', hour / 2);
    await fixture('powermove-codex-fresh', 'package', hour / 2);
    await fixture('powermove-claude-boundary', 'archive', hour);
    await fixture('powermove-claude-unrelated', null);
    await fixture('powermove-codex-unrelated', null);
    await fixture('powermove-e2e-old', 'archive');
    await fixture('other-claude-old', 'package');
    await fixture('powermove-claude-', 'archive');
    await writeFile(path.join(tmpDir, 'powermove-codex-file'), 'not a directory');

    await sweepRuntimeScratch(tmpDir, now);

    expect((await readdir(tmpDir)).sort()).toEqual([
      'other-claude-old',
      'powermove-claude-',
      'powermove-claude-boundary',
      'powermove-claude-fresh',
      'powermove-claude-unrelated',
      'powermove-codex-file',
      'powermove-codex-fresh',
      'powermove-codex-unrelated',
      'powermove-e2e-old'
    ]);
    // Repeating the sweep after an earlier cleanup is harmless.
    await sweepRuntimeScratch(tmpDir, now);
  });

  it('does not follow directory or marker symlinks or accept markers of the wrong type', async () => {
    const target = await fixture('unrelated-target', 'archive');
    await symlink(target, path.join(tmpDir, 'powermove-claude-link'));
    const linkedArchive = await fixture('powermove-claude-linked-archive', null);
    const linkedPackage = await fixture('powermove-codex-linked-package', null);
    await symlink(path.join(target, 'package.tgz'), path.join(linkedArchive, 'package.tgz'));
    await symlink(target, path.join(linkedPackage, 'package'));
    const wrongArchive = await fixture('powermove-claude-wrong-archive', null);
    const wrongPackage = await fixture('powermove-codex-wrong-package', null);
    await mkdir(path.join(wrongArchive, 'package.tgz'));
    await writeFile(path.join(wrongPackage, 'package'), 'not a directory');
    for (const dir of [linkedArchive, linkedPackage, wrongArchive, wrongPackage]) {
      await utimes(dir, new Date(now - 2 * hour), new Date(now - 2 * hour));
    }
    const before = (await readdir(tmpDir)).sort();

    await sweepRuntimeScratch(tmpDir, now);

    expect((await readdir(tmpDir)).sort()).toEqual(before);
  });
});
