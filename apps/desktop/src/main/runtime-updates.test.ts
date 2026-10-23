import { mkdir, mkdtemp, readdir, readFile, rm, symlink, utimes, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { configureRuntimeUpdates, installLatestRuntime, isNewerVersion, sweepRuntimeScratch } from './runtime-updates';

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
  const livePid = 4242;
  const deadPid = 4343;
  const alive = (pid: number) => pid === livePid;

  beforeEach(async () => { tmpDir = await mkdtemp(path.join(os.tmpdir(), 'runtime-sweep-test-')); });
  afterEach(async () => { await rm(tmpDir, { recursive: true, force: true }); });

  type Layout = { archive?: boolean; package?: boolean; owner?: string; extra?: string; age?: number; archiveAge?: number };

  async function fixture(name: string, layout: Layout = {}): Promise<string> {
    const dir = path.join(tmpDir, name);
    await mkdir(dir);
    const age = layout.age ?? 2 * hour;
    if (layout.owner !== undefined) await writeFile(path.join(dir, 'owner.pid'), layout.owner);
    if (layout.extra) await writeFile(path.join(dir, layout.extra), '{}');
    if (layout.package) {
      await mkdir(path.join(dir, 'package'));
      await utimes(path.join(dir, 'package'), new Date(now - age), new Date(now - age));
    }
    if (layout.archive) {
      const archive = path.join(dir, 'package.tgz');
      await writeFile(archive, 'partial download');
      const archiveAge = layout.archiveAge ?? age;
      await utimes(archive, new Date(now - archiveAge), new Date(now - archiveAge));
    }
    await utimes(dir, new Date(now - age), new Date(now - age));
    return dir;
  }

  const remaining = async () => (await readdir(tmpDir)).sort();

  it('reclaims runtime scratch whose owner process is gone, regardless of age', async () => {
    await fixture('powermove-runtime-claude-dead', { owner: `${deadPid}\n`, archive: true, age: 60_000 });
    await fixture('powermove-runtime-codex-dead', { owner: String(deadPid), age: 0 });
    await fixture('powermove-runtime-claude-live', { owner: String(livePid), archive: true, package: true, age: 5 * hour });

    await sweepRuntimeScratch(tmpDir, now, alive);

    expect(await remaining()).toEqual(['powermove-runtime-claude-live']);
  });

  it('falls back to an hour of inactivity when the owner is missing or unusable', async () => {
    await fixture('powermove-runtime-claude-old', { archive: true });
    await fixture('powermove-runtime-codex-empty-old', {});
    await fixture('powermove-runtime-claude-bad-owner-old', { owner: 'not a pid' });
    await fixture('powermove-runtime-codex-pid1-old', { owner: '1' });
    await fixture('powermove-runtime-claude-young', { age: hour / 2 });
    await fixture('powermove-runtime-codex-bad-owner-young', { owner: '-5', age: hour / 2 });
    // An archive still being written keeps the directory alive past the dir mtime.
    await fixture('powermove-runtime-claude-streaming', { archive: true, archiveAge: 60_000 });
    await fixture('powermove-runtime-claude-boundary', { age: hour });

    await sweepRuntimeScratch(tmpDir, now, alive);

    expect(await remaining()).toEqual([
      'powermove-runtime-claude-boundary',
      'powermove-runtime-claude-streaming',
      'powermove-runtime-claude-young',
      'powermove-runtime-codex-bad-owner-young'
    ]);
  });

  it('reclaims pre-prefix downloads only with the exact old layout and no recent activity', async () => {
    await fixture('powermove-claude-abandoned', { archive: true });
    await fixture('powermove-codex-extracted', { archive: true, package: true });
    await fixture('powermove-claude-fresh', { archive: true, age: hour / 2 });
    await fixture('powermove-codex-streaming', { archive: true, archiveAge: 60_000 });
    await fixture('powermove-claude-boundary', { archive: true, age: hour });
    // Editor-run directories share the legacy prefix: an extra file, a missing
    // archive or an owner file (which no legacy build wrote) keeps them.
    await fixture('powermove-codex-editor-inputs', { archive: true, extra: 'schema.json' });
    await fixture('powermove-claude-editor-images', { archive: true, extra: 'reference-0.png' });
    await fixture('powermove-codex-package-only', { package: true });
    await fixture('powermove-claude-empty', {});
    await fixture('powermove-claude-owner', { archive: true, owner: String(deadPid) });
    await fixture('powermove-e2e-old', { archive: true });
    await fixture('other-claude-old', { package: true });
    await fixture('powermove-claude-', { archive: true });
    await writeFile(path.join(tmpDir, 'powermove-codex-file'), 'not a directory');

    await sweepRuntimeScratch(tmpDir, now, alive);

    expect(await remaining()).toEqual([
      'other-claude-old',
      'powermove-claude-',
      'powermove-claude-boundary',
      'powermove-claude-editor-images',
      'powermove-claude-empty',
      'powermove-claude-fresh',
      'powermove-claude-owner',
      'powermove-codex-editor-inputs',
      'powermove-codex-file',
      'powermove-codex-package-only',
      'powermove-codex-streaming',
      'powermove-e2e-old'
    ]);
    // Repeating the sweep after an earlier cleanup is harmless.
    await sweepRuntimeScratch(tmpDir, now, alive);
  });

  it('does not follow directory, marker or owner symlinks or accept markers of the wrong type', async () => {
    const target = await fixture('unrelated-target', { archive: true, package: true });
    await writeFile(path.join(target, 'owner.pid'), String(deadPid));
    await symlink(target, path.join(tmpDir, 'powermove-claude-link'));
    await symlink(target, path.join(tmpDir, 'powermove-runtime-claude-link'));
    const linkedArchive = await fixture('powermove-claude-linked-archive', {});
    const linkedPackage = await fixture('powermove-codex-linked-package', { archive: true });
    const linkedOwner = await fixture('powermove-runtime-codex-linked-owner', { age: hour / 2 });
    await symlink(path.join(target, 'package.tgz'), path.join(linkedArchive, 'package.tgz'));
    await symlink(target, path.join(linkedPackage, 'package'));
    await symlink(path.join(target, 'owner.pid'), path.join(linkedOwner, 'owner.pid'));
    const wrongArchive = await fixture('powermove-claude-wrong-archive', {});
    const wrongPackage = await fixture('powermove-codex-wrong-package', { archive: true });
    await mkdir(path.join(wrongArchive, 'package.tgz'));
    await writeFile(path.join(wrongPackage, 'package'), 'not a directory');
    for (const dir of [linkedArchive, linkedPackage, linkedOwner, wrongArchive, wrongPackage]) {
      const age = dir === linkedOwner ? hour / 2 : 2 * hour;
      await utimes(dir, new Date(now - age), new Date(now - age));
    }
    const before = await remaining();

    await sweepRuntimeScratch(tmpDir, now, alive);

    expect(await remaining()).toEqual(before);
    expect((await readdir(target)).sort()).toEqual(['owner.pid', 'package', 'package.tgz']);
  });

  it('checks real process liveness by default', async () => {
    await fixture('powermove-runtime-claude-self', { owner: String(process.pid), archive: true });

    await sweepRuntimeScratch(tmpDir, now);

    expect(await remaining()).toEqual(['powermove-runtime-claude-self']);
  });
});

describe.runIf(process.platform === 'darwin')('installLatestRuntime scratch', () => {
  let tmpDir: string;

  beforeEach(async () => { tmpDir = await mkdtemp(path.join(os.tmpdir(), 'runtime-install-test-')); });
  afterEach(async () => {
    vi.unstubAllEnvs();
    vi.unstubAllGlobals();
    await rm(tmpDir, { recursive: true, force: true });
  });

  it('downloads into an owned runtime scratch directory and removes it when the install fails', async () => {
    vi.stubEnv('TMPDIR', tmpDir);
    expect(os.tmpdir()).toBe(tmpDir);
    const stale = path.join(tmpDir, 'powermove-runtime-claude-crashed');
    await mkdir(stale);
    await writeFile(path.join(stale, 'owner.pid'), '999999999');
    configureRuntimeUpdates({ root: path.join(tmpDir, 'runtimes'), appVersion: '1.0.0' });

    let during: { names: string[]; owner: string; archive: boolean } | null = null;
    vi.stubGlobal('fetch', vi.fn(async (url: string) => {
      if (url.includes('registry.npmjs.org/@anthropic-ai')) {
        return Response.json({ version: '9.9.9', dist: {
          tarball: 'https://registry.npmjs.org/claude/-/claude-9.9.9.tgz', integrity: 'sha512-x'
        } });
      }
      const names = await readdir(tmpDir);
      const scratch = names.find(name => name.startsWith('powermove-runtime-claude-'))!;
      const inside = await readdir(path.join(tmpDir, scratch));
      during = { names, owner: await readFile(path.join(tmpDir, scratch, 'owner.pid'), 'utf8'), archive: inside.includes('package.tgz') };
      return new Response(null, { status: 503 });
    }));

    await expect(installLatestRuntime('claude')).rejects.toThrow('The download failed (503).');

    // The startup sweep ran first: the crashed process's directory is gone and
    // only this install's directory existed while it downloaded.
    expect(during).not.toBeNull();
    expect(during!.names.filter(name => name.startsWith('powermove-'))).toHaveLength(1);
    expect(during!.names.some(name => /^powermove-runtime-claude-.+/.test(name))).toBe(true);
    expect(during!.owner).toBe(String(process.pid));
    expect(during!.archive).toBe(false);
    expect((await readdir(tmpDir)).filter(name => name.startsWith('powermove-'))).toEqual([]);
  });
});
