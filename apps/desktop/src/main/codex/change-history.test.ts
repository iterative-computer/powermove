import { mkdtemp, mkdir, readFile, readdir, rm, stat, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { afterEach, describe, expect, it } from 'vitest';

import {
  prepareExtensionStage,
  publishExtensionChanges,
  recoverInterruptedExtensionTransactions,
  restoreExtensionChangeSet
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
      .rejects.toThrow(/Nothing was overwritten/i);
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
