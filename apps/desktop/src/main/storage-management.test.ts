import { mkdir, mkdtemp, readFile, readdir, rm, symlink, utimes, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { STORAGE_IPC } from '../shared/storage-management';
import { applySnapshotRetention, CACHE_BUDGET_BYTES, installStorageManagement, measureStorageTree, planSnapshotRetention, StorageManagement,
  storageSettings, type StorageManagementDeps } from './storage-management';
import { applyBackupRetention, planBackupRetention } from './backup-retention';

let root: string;
const now = Date.parse('2026-10-14T12:00:00Z');
beforeEach(async () => { root = await mkdtemp(path.join(tmpdir(), 'powermove-storage-')); });
afterEach(async () => { await rm(root, { recursive: true, force: true }); });
async function file(relative: string, contents: string, date?: string): Promise<string> {
  const target = path.join(root, relative);
  await mkdir(path.dirname(target), { recursive: true });
  await writeFile(target, contents);
  if (date) {
    const time = new Date(date);
    await utimes(target, time, time);
    await utimes(path.dirname(target), time, time);
  }
  return target;
}
function deps(): StorageManagementDeps {
  let settings: unknown;
  return { root, settings: () => settings, saveSettings: async value => { settings = value; },
    cleanupBackups: async budget => applyBackupRetention(path.join(root, 'backups'), await planBackupRetention(path.join(root, 'backups'), budget, now)),
    clearCaches: vi.fn(async () => { await rm(path.join(root, 'Cache'), { recursive: true, force: true }); }),
    cacheBytes: async () => (await measureStorageTree(path.join(root, 'Cache'))).bytes,
    reveal: vi.fn(async () => {}), isTrustedSender: () => true };
}

it('estimates cleanable data and cleans only older backups and caches, preserving projects, media and accounts', async () => {
  const protectedFiles = ['store/project.one.json', 'IndexedDB/assets', 'Cookies', 'cloud/session.json', 'codex-runtime/auth.json',
    'codex-runtime/sessions/history', 'Agent Workspaces/one/artifacts/render.mp4', 'backups/one/uncertain.recovery.pmv'];
  for (const target of protectedFiles) await file(target, `preserve:${target}`);
  for (let day = 1; day <= 4; day++) await file(`backups/one/2026-08-0${day}T00-00-00-000Z.pmv`, 'snapshot');
  await file('Cache/data', 'cached bytes');
  const service = new StorageManagement(deps());
  const before = await service.status();
  expect(before.reclaimableBytes).toBe(2 * 'snapshot'.length + 'cached bytes'.length);
  expect(before.categories.find(item => item.id === 'media')!.reclaimable).toBe(0);
  const result = await service.cleanup();
  expect(result.freedBytes).toBe(before.reclaimableBytes);
  expect(result.warnings).toEqual([]);
  expect(result.status.reclaimableBytes).toBe(0);
  for (const target of protectedFiles) expect(await readFile(path.join(root, target), 'utf8')).toBe(`preserve:${target}`);
  expect(await readdir(path.join(root, 'backups/one'))).toEqual(expect.arrayContaining(['2026-08-03T00-00-00-000Z.pmv', '2026-08-04T00-00-00-000Z.pmv']));
});

it('retains two recovery snapshots per family and uses a longer automatic grace period', async () => {
  for (const family of ['restart-backups/', 'store-backup-']) {
    for (let day = 1; day <= 4; day++) await file(`${family}snapshot${day}/data`, 'recovery', `2026-09-${day + 10}T12:00:00Z`);
  }
  const automatic = await planSnapshotRetention(root, false, now);
  expect(automatic).toHaveLength(4);
  const manual = await planSnapshotRetention(root, true, now);
  expect(manual).toHaveLength(4);
  expect(await applySnapshotRetention(root, manual)).toBe(4 * 'recovery'.length);
  expect(await readdir(path.join(root, 'restart-backups'))).toEqual(['snapshot3', 'snapshot4']);
  expect((await readdir(root)).filter(name => name.startsWith('store-backup-'))).toEqual(['store-backup-snapshot3', 'store-backup-snapshot4']);
  // Once all remaining snapshots are 20–24 days old, only explicit cleanup trims them.
  await file('restart-backups/snapshot3/data', 'recovery', '2026-09-20T12:00:00Z');
  await file('restart-backups/snapshot4/data', 'recovery', '2026-09-21T12:00:00Z');
  await file('restart-backups/newest/data', 'recovery', '2026-09-24T12:00:00Z');
  expect(await planSnapshotRetention(root, false, now)).toEqual([]);
  expect((await planSnapshotRetention(root, true, now)).map(item => path.basename(item.file))).toEqual(['snapshot3']);
});

it('measures and cleans snapshots without following links into shared resources', async () => {
  const outside = await mkdtemp(path.join(tmpdir(), 'powermove-shared-'));
  try {
    await writeFile(path.join(outside, 'keep'), 'shared resources');
    await symlink(outside, path.join(root, 'shared'));
    expect((await measureStorageTree(root)).bytes).toBe(0);
    await symlink(outside, path.join(root, 'restart-backups'));
    expect(await planSnapshotRetention(root, true, now)).toEqual([]);
    expect(await readFile(path.join(outside, 'keep'), 'utf8')).toBe('shared resources');
  } finally { await rm(outside, { recursive: true, force: true }); }
});

it('skips a recovery snapshot modified after inspection', async () => {
  for (let day = 1; day <= 3; day++) await file(`restart-backups/${day}/data`, 'old', `2026-08-0${day}T00:00:00Z`);
  const plan = await planSnapshotRetention(root, true, now);
  await writeFile(path.join(plan[0]!.file, 'data'), 'new recovery data');
  expect(await applySnapshotRetention(root, plan)).toBe(0);
});

it('honors automatic cleanup opt-out and validates settings', async () => {
  const options = deps(), service = new StorageManagement(options);
  await service.configure({ automatic: false, backupBudgetGiB: 1 });
  await file('Cache/data', 'temporary');
  await service.maintain();
  expect(options.clearCaches).not.toHaveBeenCalled();
  expect((await service.status()).settings).toEqual({ automatic: false, backupBudgetGiB: 1 });
  for (const raw of [null, { automatic: 'yes', backupBudgetGiB: 5 }, { automatic: true, backupBudgetGiB: -1 }, { automatic: true, backupBudgetGiB: Infinity }]) {
    await expect(service.configure(raw)).rejects.toThrow('Invalid storage');
  }
  expect(storageSettings(null)).toEqual({ automatic: true, backupBudgetGiB: 5 });
});

it('automatically removes expired data and clears caches only above their budget', async () => {
  const options = deps();
  const service = new StorageManagement(options);
  for (let day = 1; day <= 4; day++) {
    await file(`backups/one/2026-08-0${day}T00-00-00-000Z.pmv`, 'snapshot');
    await file(`restart-backups/${day}/data`, 'old recovery', `2026-08-0${day}T00:00:00Z`);
  }
  await service.maintain();
  expect(await readdir(path.join(root, 'backups/one'))).toHaveLength(2);
  expect(await readdir(path.join(root, 'restart-backups'))).toEqual(['3', '4']);
  expect(options.clearCaches).not.toHaveBeenCalled();
  options.cacheBytes = async () => CACHE_BUDGET_BYTES + 1;
  await service.maintain();
  expect(options.clearCaches).toHaveBeenCalledOnce();
});

it('aborts cleanup on a busy save before touching recovery snapshots or caches', async () => {
  const options = deps();
  options.cleanupBackups = async () => { throw new Error('A project is saving.'); };
  for (let day = 1; day <= 3; day++) await file(`restart-backups/${day}/data`, 'recovery', `2026-08-0${day}T00:00:00Z`);
  await expect(new StorageManagement(options).cleanup()).rejects.toThrow('saving');
  expect(options.clearCaches).not.toHaveBeenCalled();
  expect(await readdir(path.join(root, 'restart-backups'))).toHaveLength(3);
});

it('guards all IPC handlers and schedules maintenance with disposable timers', async () => {
  vi.useFakeTimers();
  const handlers = new Map<string, Function>();
  const options = deps();
  options.isTrustedSender = event => (event as any).trusted === true;
  const installed = installStorageManagement({ handle: (channel, handler) => { handlers.set(channel, handler); } }, options);
  try {
    for (const channel of [STORAGE_IPC.status, STORAGE_IPC.configure, STORAGE_IPC.cleanup, STORAGE_IPC.reveal]) await expect(Promise.resolve().then(() => handlers.get(channel)!({ trusted: false }))).rejects.toThrow('Unauthorized');
    expect(vi.getTimerCount()).toBe(2);
    installed.dispose();
    expect(vi.getTimerCount()).toBe(0);
  } finally { installed.dispose(); vi.useRealTimers(); }
});
