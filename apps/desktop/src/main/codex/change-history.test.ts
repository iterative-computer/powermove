import { mkdtemp, mkdir, readFile, readdir, rename, rm, stat, symlink, writeFile } from 'node:fs/promises';
import { execFileSync } from 'node:child_process';
import os from 'node:os';
import path from 'node:path';

import { afterEach, describe, expect, it, vi } from 'vitest';

// Runs once, around main's listing of the folder it names: where a racing
// agent process would swap that folder for a link.
type Swap = { directory: string; before: () => Promise<void>; after?: () => Promise<void> };
const listing = vi.hoisted(() => ({ swap: null as null | Swap }));
vi.mock('node:fs/promises', async (importOriginal) => {
  const actual = await importOriginal<typeof import('node:fs/promises')>();
  const readdir = (async (directory: string, ...rest: unknown[]) => {
    const swap = listing.swap;
    const swapping = swap && path.resolve(String(directory)) === swap.directory;
    if (swapping) { listing.swap = null; await swap.before(); }
    const entries = await (actual.readdir as (...args: unknown[]) => Promise<unknown>)(directory, ...rest);
    if (swapping) await swap.after?.();
    return entries;
  }) as typeof actual.readdir;
  return { ...actual, readdir, default: { ...actual, readdir } };
});

import {
  prepareExtensionStage,
  publishExtensionChanges,
  recoverAllInterruptedExtensionTransactions,
  recoverInterruptedExtensionTransactions,
  restoreExtensionChangeSet,
  withStageSnapshot
} from './change-history';

const temporaryDirectories: string[] = [];

async function temporaryDirectory(): Promise<string> {
  const directory = await mkdtemp(path.join(os.tmpdir(), 'powermove-change-history-'));
  temporaryDirectories.push(directory);
  return directory;
}

afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map((directory) => rm(directory, { recursive: true, force: true })));
});

async function extension(root: string, id: string, source: string): Promise<void> {
  const directory = path.join(root, id);
  await mkdir(directory, { recursive: true });
  await writeFile(path.join(directory, 'manifest.json'), JSON.stringify({
    id, name: id, version: '1.0.0', apiVersion: 1, entry: 'index.ts', author: 'agent'
  }));
  await writeFile(path.join(directory, 'index.ts'), source);
}

async function stage(root: string, runId = 'run-1') {
  const liveDirectory = path.join(root, 'extensions');
  const stagingDirectory = path.join(root, 'workspace', runId);
  const historyRoot = path.join(root, 'history');
  await mkdir(liveDirectory, { recursive: true });
  return prepareExtensionStage({ liveDirectory, stagingDirectory, historyRoot, projectId: 'project-1', runId });
}

describe('agent extension isolation and recovery', () => {
  it('keeps staged edits out of the live app until a validated atomic promotion', async () => {
    const root = await temporaryDirectory();
    const live = path.join(root, 'extensions');
    await extension(live, 'existing-change', 'export default "before";');
    const prepared = await stage(root);
    const liveRootBefore = await stat(live);

    await writeFile(path.join(prepared.stagingDirectory, 'existing-change', 'index.ts'), 'export default "after";');
    await extension(prepared.stagingDirectory, 'new-change', 'export default "new";');
    expect(await readFile(path.join(live, 'existing-change', 'index.ts'), 'utf8')).toContain('before');
    await expect(readFile(path.join(live, 'new-change', 'index.ts'), 'utf8')).rejects.toMatchObject({ code: 'ENOENT' });

    const record = await publishExtensionChanges(prepared, [
      { id: 'existing-change', action: 'updated', summary: 'Updated safely' },
      { id: 'new-change', action: 'created', summary: 'Created safely' }
    ]);
    expect(record?.id).toBe('run-1');
    expect(await readFile(path.join(live, 'existing-change', 'index.ts'), 'utf8')).toContain('after');
    expect(await readFile(path.join(live, 'new-change', 'index.ts'), 'utf8')).toContain('new');
    expect((await stat(live)).ino).toBe(liveRootBefore.ino);
    expect(await readFile(path.join(prepared.historyRoot, 'run-1', 'before', 'existing-change', 'index.ts'), 'utf8'))
      .toContain('before');
  });

  it('rejects hidden, mislabeled, and conflicting edits without overwriting live files', async () => {
    const root = await temporaryDirectory();
    const live = path.join(root, 'extensions');
    await extension(live, 'safe-change', 'export default "before";');
    const hidden = await stage(root, 'hidden');
    await writeFile(path.join(hidden.stagingDirectory, 'safe-change', 'index.ts'), 'export default "hidden";');
    await expect(publishExtensionChanges(hidden, [])).rejects.toThrow(/did not match/i);
    await expect(publishExtensionChanges(hidden, [])).rejects.toThrow('Actual staged extension changes: [{"id":"safe-change","action":"updated"}]');
    expect(await readFile(path.join(live, 'safe-change', 'index.ts'), 'utf8')).toContain('before');

    const conflict = await stage(root, 'conflict');
    await writeFile(path.join(conflict.stagingDirectory, 'safe-change', 'index.ts'), 'export default "agent";');
    await writeFile(path.join(live, 'safe-change', 'index.ts'), 'export default "user";');
    await expect(publishExtensionChanges(conflict, [{ id: 'safe-change', action: 'updated' }]))
      .rejects.toThrow(/No extension files were overwritten/i);
    expect(await readFile(path.join(live, 'safe-change', 'index.ts'), 'utf8')).toContain('user');
  });

  it('restores the complete pre-run extension state and refuses to erase newer work', async () => {
    const root = await temporaryDirectory();
    const live = path.join(root, 'extensions');
    await extension(live, 'recoverable-change', 'export default "before";');
    const prepared = await stage(root);
    await writeFile(path.join(prepared.stagingDirectory, 'recoverable-change', 'index.ts'), 'export default "after";');
    await publishExtensionChanges(prepared, [{ id: 'recoverable-change', action: 'updated' }]);

    await restoreExtensionChangeSet({ liveDirectory: live, historyRoot: prepared.historyRoot, changeSetId: 'run-1' });
    expect(await readFile(path.join(live, 'recoverable-change', 'index.ts'), 'utf8')).toContain('before');

    const second = await stage(root, 'run-2');
    await writeFile(path.join(second.stagingDirectory, 'recoverable-change', 'index.ts'), 'export default "newer";');
    await publishExtensionChanges(second, [{ id: 'recoverable-change', action: 'updated' }]);
    await expect(restoreExtensionChangeSet({ liveDirectory: live, historyRoot: prepared.historyRoot, changeSetId: 'run-1' }))
      .rejects.toThrow(/Newer extension changes/i);
  });

  it('repairs a transaction interrupted after one live extension was moved aside', async () => {
    const root = await temporaryDirectory();
    const live = path.join(root, 'extensions');
    const history = path.join(root, 'history');
    const pending = path.join(history, '.pending-crash');
    await extension(live, 'safe-change', 'export default "safe";');
    await mkdir(path.join(pending, 'before'), { recursive: true });
    await import('node:fs/promises').then(({ rename }) =>
      rename(path.join(live, 'safe-change'), path.join(pending, 'before', 'safe-change')));
    await writeFile(path.join(pending, 'change-set.json'), JSON.stringify({
      version: 1,
      id: 'crash',
      projectId: 'project-1',
      createdAt: new Date().toISOString(),
      beforeRootHash: 'before',
      afterRootHash: 'after',
      changes: [{ id: 'safe-change', action: 'updated' }]
    }));

    await recoverInterruptedExtensionTransactions(history, live);
    expect(await readFile(path.join(live, 'safe-change', 'index.ts'), 'utf8')).toContain('safe');
    expect(await readdir(history)).toEqual([]);
  });
});

// Simulate process death between the filesystem operations of publication and
// rollback, including an earlier entry already restored before a second crash.
it.each(['prepared', 'moved', 'installed', 'recovering', 'restored'])('recovers mixed entries at %s and is repeatable', async (phase) => {
  const root = await temporaryDirectory();
  const live = path.join(root, 'extensions');
  const history = path.join(root, 'history');
  const pending = path.join(history, '.pending-mixed');
  await extension(live, 'untouched', 'original');
  await extension(live, 'updated', 'original');
  await extension(live, 'removed', 'original');
  await extension(path.join(pending, 'next'), 'created', 'new');
  await extension(path.join(pending, 'next'), 'updated', 'new');
  await mkdir(path.join(pending, 'before'), { recursive: true });
  const { rename, cp } = await import('node:fs/promises');
  if (phase !== 'prepared') {
    for (const id of ['updated', 'removed']) await rename(path.join(live, id), path.join(pending, 'before', id));
  }
  if (['installed', 'recovering', 'restored'].includes(phase)) {
    for (const id of ['updated', 'created']) await rename(path.join(pending, 'next', id), path.join(live, id));
  }
  if (phase === 'recovering') {
    await rm(path.join(live, 'updated'), { recursive: true });
    await extension(path.join(pending, 'restore'), 'updated', 'partial recovery');
  }
  if (phase === 'restored') {
    await rm(path.join(live, 'updated'), { recursive: true });
    await cp(path.join(pending, 'before', 'updated'), path.join(live, 'updated'), { recursive: true });
  }
  await writeFile(path.join(pending, 'change-set.json'), JSON.stringify({ changes: [
    { id: 'updated', action: 'updated' }, { id: 'created', action: 'created' },
    { id: 'removed', action: 'removed' }, { id: 'untouched', action: 'updated' }
  ] }));
  await recoverInterruptedExtensionTransactions(history, live);
  await recoverInterruptedExtensionTransactions(history, live);
  for (const id of ['updated', 'removed', 'untouched']) {
    expect(await readFile(path.join(live, id, 'index.ts'), 'utf8')).toBe('original');
  }
  await expect(stat(path.join(live, 'created'))).rejects.toMatchObject({ code: 'ENOENT' });
  expect(await readdir(history)).toEqual([]);
});

it('does not delete untouched later extensions when restoring a snapshot fails', async () => {
  const root = await temporaryDirectory();
  const live = path.join(root, 'extensions');
  for (const id of ['first', 'later']) await extension(live, id, 'before');
  const prepared = await stage(root);
  for (const id of ['first', 'later']) await extension(prepared.stagingDirectory, id, 'after');
  await publishExtensionChanges(prepared, [
    { id: 'first', action: 'updated' }, { id: 'later', action: 'updated' }
  ]);
  const { symlink } = await import('node:fs/promises');
  await symlink('/invalid-target', path.join(prepared.historyRoot, 'run-1', 'before', 'first', 'invalid'));
  await expect(restoreExtensionChangeSet({ liveDirectory: live, historyRoot: prepared.historyRoot, changeSetId: 'run-1' }))
    .rejects.toThrow(/symbolic/i);
  for (const id of ['first', 'later']) expect(await readFile(path.join(live, id, 'index.ts'), 'utf8')).toBe('after');
});

it('publishes the private copy it checked even when the stage changes afterwards', async () => {
  const root = await temporaryDirectory();
  const live = path.join(root, 'extensions');
  await extension(live, 'checked', 'before');
  const prepared = await stage(root);
  await writeFile(path.join(prepared.stagingDirectory, 'checked', 'index.ts'), 'checked');
  // Outside the extension folders: not copied, not refused.
  await symlink('/etc', path.join(prepared.stagingDirectory, 'stray-link'));
  const record = await withStageSnapshot(prepared, async (snapshot) => {
    expect(path.relative(prepared.historyRoot, snapshot.stagingDirectory).startsWith('..')).toBe(false);
    expect(await readFile(path.join(snapshot.stagingDirectory, 'checked', 'index.ts'), 'utf8')).toBe('checked');
    // A process that outlived its command rewrites the stage after the check.
    await writeFile(path.join(prepared.stagingDirectory, 'checked', 'index.ts'), 'late');
    await extension(prepared.stagingDirectory, 'smuggled', 'late');
    return publishExtensionChanges(snapshot, [{ id: 'checked', action: 'updated' }]);
  });
  expect(record?.changes).toEqual([{ id: 'checked', action: 'updated' }]);
  expect(await readFile(path.join(live, 'checked', 'index.ts'), 'utf8')).toBe('checked');
  await expect(stat(path.join(live, 'smuggled'))).rejects.toMatchObject({ code: 'ENOENT' });
  expect((await readdir(prepared.historyRoot)).filter((name) => name.startsWith('.snapshot-'))).toEqual([]);
});

it('snapshots only changed and reported extensions and publishes the rest as their baseline', async () => {
  const root = await temporaryDirectory();
  const live = path.join(root, 'extensions');
  for (const id of ['kept', 'reported', 'edited', 'removed']) await extension(live, id, 'before');
  const prepared = await stage(root);
  await writeFile(path.join(prepared.stagingDirectory, 'edited', 'index.ts'), 'after');
  await rm(path.join(prepared.stagingDirectory, 'removed'), { recursive: true });
  await extension(prepared.stagingDirectory, 'created', 'new');
  const record = await withStageSnapshot(prepared, async (snapshot) => {
    expect((await readdir(snapshot.stagingDirectory)).sort()).toEqual(['created', 'edited', 'reported']);
    expect([...snapshot.unchanged]).toEqual(['kept']);
    return publishExtensionChanges(snapshot, [
      { id: 'created', action: 'created' }, { id: 'edited', action: 'updated' }, { id: 'removed', action: 'removed' }
    ]);
  }, ['created', 'edited', 'removed', 'reported']);
  expect(record?.changes.map((change) => change.id)).toEqual(['created', 'edited', 'removed']);
  expect((await readdir(live)).sort()).toEqual(['created', 'edited', 'kept', 'reported']);
  expect(await readFile(path.join(live, 'kept', 'index.ts'), 'utf8')).toBe('before');
  expect(await readFile(path.join(live, 'edited', 'index.ts'), 'utf8')).toBe('after');
  // A report naming an unchanged extension is checked against its copy and refused.
  await expect(withStageSnapshot(await stage(root, 'run-2'), (snapshot) =>
    publishExtensionChanges(snapshot, [{ id: 'kept', action: 'updated' }]), ['kept'])).rejects.toThrow('did not match');
});

it.each(['left as a link', 'swapped back'])('refuses to snapshot a folder swapped for a link to private files before its listing, %s', async (mode) => {
  const root = await temporaryDirectory();
  const live = path.join(root, 'extensions');
  await extension(live, 'checked', 'before');
  const prepared = await stage(root);
  await mkdir(path.join(prepared.stagingDirectory, 'checked', 'assets'));
  await writeFile(path.join(prepared.stagingDirectory, 'checked', 'assets', 'logo.txt'), 'logo');
  const secrets = path.join(root, 'home', '.aws');
  await mkdir(secrets, { recursive: true });
  await writeFile(path.join(secrets, 'credentials'), 'aws_secret_access_key = private');
  const assets = path.join(prepared.stagingDirectory, 'checked', 'assets');
  listing.swap = {
    directory: assets,
    before: async () => { await rename(assets, `${assets}-real`); await symlink(secrets, assets); },
    ...(mode === 'swapped back' ? { after: async () => { await rm(assets); await rename(`${assets}-real`, assets); } } : {})
  };
  let copied: string[] = [];
  await expect(withStageSnapshot(prepared, async (snapshot) => {
    copied = await readdir(path.join(snapshot.stagingDirectory, 'checked', 'assets'));
  }, ['checked'])).rejects.toThrow(/changed while it was being copied|ENOENT/);
  expect(listing.swap).toBeNull();
  expect(copied).toEqual([]);
  expect((await readdir(prepared.historyRoot)).filter((name) => name.startsWith('.snapshot-'))).toEqual([]);
});

it.each(['a FIFO', 'a link to /dev/zero'])('refuses a stage file swapped for %s after its listing without waiting or buffering', async (kind) => {
  const root = await temporaryDirectory();
  const live = path.join(root, 'extensions');
  await extension(live, 'checked', 'before');
  const prepared = await stage(root);
  const folder = path.join(prepared.stagingDirectory, 'checked');
  const file = path.join(folder, 'index.ts');
  listing.swap = {
    directory: folder,
    before: async () => undefined,
    after: async () => {
      await rm(file);
      if (kind === 'a FIFO') execFileSync('/usr/bin/mkfifo', [file]);
      else await symlink('/dev/zero', file);
    }
  };
  // Unreported and unchanged when listed, so only the baseline hash reads it.
  await expect(withStageSnapshot(prepared, async () => undefined)).rejects.toThrow(/unsupported (file|symbolic link): index\.ts/);
  expect(listing.swap).toBeNull();
}, 5_000);

it('clears snapshots a crash left behind at boot', async () => {
  const userData = await temporaryDirectory();
  const leftover = path.join(userData, 'Agent Change History', 'project-1', '.snapshot-run-1-crashed');
  await mkdir(path.join(leftover, 'stage'), { recursive: true });
  await recoverAllInterruptedExtensionTransactions(userData, path.join(userData, 'extensions'));
  await expect(stat(leftover)).rejects.toMatchObject({ code: 'ENOENT' });
});
