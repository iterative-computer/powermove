import { mkdtemp, mkdir, readFile, readdir, rm, stat, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { afterEach, expect, it, vi } from 'vitest';

const native = vi.hoisted(() => ({ afterRename: null as null | ((from: string, to: string) => Promise<void>) }));
vi.mock('node:fs/promises', async importOriginal => {
  const fs = await importOriginal<typeof import('node:fs/promises')>();
  return { ...fs, rename: async (from: string, to: string) => { await fs.rename(from, to); await native.afterRename?.(from, to); } };
});
import {
  prepareExtensionStage, publishExtensionChanges, recoverAllInterruptedExtensionTransactions,
  recoverInterruptedExtensionTransactions, restoreExtensionChangeSet
} from './change-history';

afterEach(() => { native.afterRename = null; });

const id = 'first-change';
const changes = [{ id, action: 'updated' as const, summary: 'Update extension' }];

async function fixture() {
  const root = await mkdtemp(path.join(os.tmpdir(), 'powermove-extension-transactions-'));
  const liveDirectory = path.join(root, 'extensions');
  const historyRoot = path.join(root, 'Agent Change History', 'project-one');
  await mkdir(path.join(liveDirectory, id), { recursive: true });
  await writeFile(path.join(liveDirectory, id, 'manifest.json'), JSON.stringify({ id, name: id, version: '1.0.0', apiVersion: 1, entry: 'index.ts', author: 'agent' }));
  await writeFile(path.join(liveDirectory, id, 'index.ts'), 'export default "initial";');
  const priorStage = await prepareExtensionStage({ liveDirectory, historyRoot, stagingDirectory: path.join(root, 'prior-stage'), projectId: 'project-one', runId: 'prior' });
  await writeFile(path.join(priorStage.stagingDirectory, id, 'index.ts'), 'export default "before";');
  const prior = await publishExtensionChanges(priorStage, changes);
  const stage = await prepareExtensionStage({ liveDirectory, historyRoot, stagingDirectory: path.join(root, 'current-stage'), projectId: 'project-one', runId: 'current' });
  await writeFile(path.join(stage.stagingDirectory, id, 'index.ts'), 'export default "after";');
  return { root, liveDirectory, historyRoot, stage, prior: prior! };
}

function holdPublication(f: Awaited<ReturnType<typeof fixture>>) {
  let resume!: () => void;
  const pause = new Promise<void>(resolve => { resume = resolve; });
  let ready!: () => void;
  const held = new Promise<void>(resolve => { ready = resolve; });
  let journal = '';
  native.afterRename = async (from, to) => {
    if (String(from) === path.join(f.liveDirectory, id) && String(to).endsWith(path.join('before', id))) {
      journal = path.dirname(path.dirname(String(to)));
      ready(); await pause;
    }
  };
  const publication = publishExtensionChanges(f.stage, changes).then(value => ({ value }), error => ({ error }));
  return { resume, held, publication, journal: () => journal };
}

it.each(['prepare', 'recover', 'recover-all', 'restore'])('does not treat an active publication as interrupted during %s', async operation => {
  const f = await fixture();
  const h = holdPublication(f);
  let waiting: Promise<any> | undefined;
  try {
    await h.held;
    // The tree is deliberately halfway through promotion: the old folder is
    // in its real journal and the replacement has not yet been moved live.
    const work = operation === 'prepare'
      ? prepareExtensionStage({ ...f.stage, liveDirectory: `${f.liveDirectory}${path.sep}.`, stagingDirectory: path.join(f.root, 'next-stage'), runId: 'next' })
      : operation === 'recover' ? recoverInterruptedExtensionTransactions(f.historyRoot, f.liveDirectory)
        : operation === 'recover-all' ? recoverAllInterruptedExtensionTransactions(f.root, f.liveDirectory)
          : restoreExtensionChangeSet({ ...f, changeSetId: f.prior.id });
    waiting = work.then(value => ({ value }), error => ({ error }));
    await new Promise(resolve => setTimeout(resolve, 100));
    expect(await stat(path.join(f.liveDirectory, id)).catch(() => null)).toBeNull();
    expect((await stat(h.journal())).isDirectory()).toBe(true);
    h.resume();
    expect(await h.publication).toMatchObject({ value: { id: 'current' } });
    const completed = await waiting;
    if (operation === 'restore') expect(completed).toMatchObject({ error: expect.objectContaining({ message: expect.stringContaining('Newer extension changes exist') }) });
    else expect(completed).toHaveProperty('value');
    expect(await readFile(path.join(f.liveDirectory, id, 'index.ts'), 'utf8')).toBe('export default "after";');
    if (operation === 'prepare') expect(await readFile(path.join(completed.value.stagingDirectory, id, 'index.ts'), 'utf8')).toBe('export default "after";');
    expect((await readdir(f.historyRoot)).some(name => name.startsWith('.pending-'))).toBe(false);
  } finally {
    h.resume(); await h.publication; await waiting;
    await rm(f.root, { recursive: true, force: true });
  }
});

it('allows a different live directory to publish while another tree is held', async () => {
  const first = await fixture(), second = await fixture();
  const h = holdPublication(first);
  let independent: Promise<any> | undefined;
  let outcome: any;
  try {
    await h.held;
    independent = publishExtensionChanges(second.stage, changes).then(value => { outcome = { value }; }, error => { outcome = { error }; });
    await vi.waitFor(() => expect(outcome).toMatchObject({ value: { id: 'current' } }));
    expect(await readFile(path.join(second.liveDirectory, id, 'index.ts'), 'utf8')).toBe('export default "after";');
    expect(await stat(path.join(first.liveDirectory, id)).catch(() => null)).toBeNull();
    h.resume();
    expect(await h.publication).toMatchObject({ value: { id: 'current' } });
  } finally {
    h.resume(); await h.publication; await independent;
    await Promise.all([first, second].map(f => rm(f.root, { recursive: true, force: true })));
  }
});
