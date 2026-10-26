import { mkdir, mkdtemp, open, readFile, readdir, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { afterEach, beforeEach, expect, it } from 'vitest';
import { applyBackupRetention, deduplicateBackups, planBackupRetention } from './backup-retention';

let root: string;
const now = Date.parse('2026-10-14T12:00:00Z');
const name = (day: number, month = '10') => `2026-${month}-${String(day).padStart(2, '0')}T00-00-00-000Z.pmv`;
async function backup(id: string, day: number, size: number, month = '10'): Promise<string> {
  const directory = path.join(root, id);
  await mkdir(directory, { recursive: true });
  const file = path.join(directory, name(day, month));
  const handle = await open(file, 'w');
  try { await handle.truncate(size); } finally { await handle.close(); }
  return file;
}
beforeEach(async () => { root = await mkdtemp(path.join(tmpdir(), 'powermove-backup-budget-')); });
afterEach(async () => { await rm(root, { recursive: true, force: true }); });

it('enforces a shared budget oldest first while keeping two backups for every project', async () => {
  for (let day = 1; day <= 5; day++) await backup('one', day, 10);
  for (let day = 10; day <= 12; day++) await backup('two', day, 10);
  const plan = await planBackupRetention(root, 50, now);
  expect(plan.totalBytes).toBe(80);
  expect(plan.reclaimableBytes).toBe(30);
  expect(plan.remove.map(item => path.basename(item.file))).toEqual([name(1), name(2), name(3)]);
  expect(await applyBackupRetention(root, plan)).toBe(30);
  expect(await readdir(path.join(root, 'one'))).toEqual([name(4), name(5)]);
  expect(await readdir(path.join(root, 'two'))).toHaveLength(3);
});

it('keeps the newest two even when they are old and exceed the budget, preserving recovery and manual files', async () => {
  for (let day = 1; day <= 4; day++) await backup('one', day, 100, '08');
  await writeFile(path.join(root, 'one', 'manual.pmv'), 'manual');
  await writeFile(path.join(root, 'one', name(1, '08') + '.recovery.pmv'), 'recovery');
  const plan = await planBackupRetention(root, 1, now);
  expect(plan.retainedBytes).toBe(200);
  expect(await applyBackupRetention(root, plan)).toBe(200);
  expect(await readdir(path.join(root, 'one'))).toEqual(expect.arrayContaining([name(3, '08'), name(4, '08'), 'manual.pmv', name(1, '08') + '.recovery.pmv']));
});

it('retains five recent generations under budget and trims older ones', async () => {
  for (let day = 1; day <= 7; day++) await backup('one', day, 10);
  expect((await planBackupRetention(root, 100, now)).remove.map(item => path.basename(item.file))).toEqual([name(2), name(1)]);
});

it('orders colliding timestamp generations numerically', async () => {
  await backup('one', 1, 10);
  await writeFile(path.join(root, 'one', name(1) + '-9'), 'older');
  await writeFile(path.join(root, 'one', name(1) + '-10'), 'newer');
  expect((await planBackupRetention(root, 0, now)).remove.map(item => path.basename(item.file))).toEqual([name(1)]);
});

it('does not traverse symlinked roots, project folders or files', async () => {
  const outside = await mkdtemp(path.join(tmpdir(), 'powermove-outside-backups-'));
  try {
    await writeFile(path.join(outside, name(1)), 'keep');
    await symlink(outside, path.join(root, 'linked-project'));
    await mkdir(path.join(root, 'one'));
    await symlink(path.join(outside, name(1)), path.join(root, 'one', name(1)));
    expect((await planBackupRetention(root, 0, now)).totalBytes).toBe(0);
    const linkedRoot = path.join(root, 'linked-root');
    await symlink(outside, linkedRoot);
    expect((await planBackupRetention(linkedRoot, 0, now)).remove).toEqual([]);
  } finally { await rm(outside, { recursive: true, force: true }); }
});

it('skips files changed since inspection and rejects paths outside its root', async () => {
  for (let day = 1; day <= 3; day++) await backup('one', day, 10);
  const plan = await planBackupRetention(root, 0, now);
  await writeFile(plan.remove[0]!.file, 'new version of old file');
  expect(await applyBackupRetention(root, plan)).toBe(0);
  const modified = { ...plan.remove[0]!, file: path.join(root, '..', 'outside.pmv') };
  expect(await applyBackupRetention(root, { ...plan, remove: [modified] })).toBe(0);
});

it('upgrade cleanup removes only duplicate successful backups and preserves distinct work, recovery and manual copies', async () => {
  const first = await backup('one', 1, 3), second = await backup('one', 2, 3), newest = await backup('one', 3, 3);
  await writeFile(first, 'old'); await writeFile(second, 'new'); await writeFile(newest, 'new');
  await writeFile(path.join(root, 'one', 'manual.pmv'), 'new');
  await writeFile(path.join(root, 'one', name(4) + '.recovery.pmv'), 'new');
  expect(await deduplicateBackups(root)).toBe(3);
  expect(await readFile(first, 'utf8')).toBe('old'); expect(await readFile(newest, 'utf8')).toBe('new');
  expect(await readdir(path.join(root, 'one'))).toEqual(expect.arrayContaining([name(1), name(3), 'manual.pmv', name(4) + '.recovery.pmv']));
  expect(await deduplicateBackups(root)).toBe(0);
});
