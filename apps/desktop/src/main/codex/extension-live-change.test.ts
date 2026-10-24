import { mkdtemp, mkdir, readFile, readdir, rm, stat, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import { prepareExtensionStage, publishExtensionChanges, restoreExtensionChangeSet, withStageSnapshot } from './change-history';

const temporaryDirectories: string[] = [];
afterEach(async () => { await Promise.all(temporaryDirectories.splice(0).map(directory => rm(directory, { recursive: true, force: true }))); });
async function temporaryDirectory() {
  const directory = await mkdtemp(path.join(os.tmpdir(), 'powermove-extension-live-change-'));
  temporaryDirectories.push(directory);
  return directory;
}
async function extension(root: string, id: string, source: string) {
  await mkdir(path.join(root, id), { recursive: true });
  await writeFile(path.join(root, id, 'manifest.json'), JSON.stringify({ id, name: id, version: '1.0.0', apiVersion: 1, entry: 'index.ts', author: 'agent' }));
  await writeFile(path.join(root, id, 'index.ts'), source);
}
async function stage(root: string, runId = 'run-1') {
  const liveDirectory = path.join(root, 'extensions');
  await mkdir(liveDirectory, { recursive: true });
  return prepareExtensionStage({ liveDirectory, stagingDirectory: path.join(root, 'stage', runId), historyRoot: path.join(root, 'history'), projectId: 'project-1', runId });
}

describe('extensions changing during agent runs', () => {
  it('finishes a project-only run after unrelated extensions were added, updated or removed', async () => {
    const root = await temporaryDirectory();
    const live = path.join(root, 'extensions');
    await extension(live, 'updated', 'before');
    await extension(live, 'removed', 'before');
    const prepared = await stage(root);
    await extension(live, 'added', 'new');
    await extension(live, 'updated', 'after');
    await rm(path.join(live, 'removed'), { recursive: true });
    // The same stage may be retained by a cancelled run's checkpoint. Neither
    // completion nor another attempt should treat its old catalog as edits.
    for (let attempt = 0; attempt < 3; attempt++) {
      await expect(withStageSnapshot(prepared, snapshot => publishExtensionChanges(snapshot, []))).resolves.toBeNull();
    }
    expect(await readFile(path.join(live, 'updated', 'index.ts'), 'utf8')).toBe('after');
    expect(await readFile(path.join(live, 'added', 'index.ts'), 'utf8')).toBe('new');
    await expect(stat(path.join(live, 'removed'))).rejects.toMatchObject({ code: 'ENOENT' });
    expect(await readdir(prepared.historyRoot)).toEqual([]);
  });

  it('publishes only its edits and records the current live baseline for guarded restore', async () => {
    const root = await temporaryDirectory();
    const live = path.join(root, 'extensions');
    await extension(live, 'agent-edit', 'before');
    await extension(live, 'user-edit', 'before');
    const prepared = await stage(root);
    await extension(prepared.stagingDirectory, 'agent-edit', 'agent');
    await extension(live, 'user-edit', 'user');
    await extension(live, 'user-added', 'keep');
    const record = await publishExtensionChanges(prepared, [{ id: 'agent-edit', action: 'updated' }]);
    expect(record?.beforeRootHash).not.toBe(prepared.baselineRootHash);
    expect(await readFile(path.join(live, 'user-edit', 'index.ts'), 'utf8')).toBe('user');
    await restoreExtensionChangeSet({ liveDirectory: live, historyRoot: prepared.historyRoot, changeSetId: 'run-1' });
    expect(await readFile(path.join(live, 'agent-edit', 'index.ts'), 'utf8')).toBe('before');
    expect(await readFile(path.join(live, 'user-edit', 'index.ts'), 'utf8')).toBe('user');
    expect(await readFile(path.join(live, 'user-added', 'index.ts'), 'utf8')).toBe('keep');
  });

});
