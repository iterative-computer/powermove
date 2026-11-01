import { execFileSync, spawn } from 'node:child_process';
import { mkdtemp, readFile, rm, stat, symlink, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { afterEach, beforeAll, beforeEach, afterAll, expect, it } from 'vitest';
import { TransitionLock, projectLockPath } from './lock';
let build: string, root: string;
const leases: TransitionLock[] = [];
beforeAll(async () => {
  build = await mkdtemp(path.join(os.tmpdir(), 'pm-tr-helper-'));
  const executable = path.join(build, 'lock');
  execFileSync('cc', ['-std=c11', '-O2', '-Wall', '-Wextra', '-Werror', path.resolve('native/transition-lock.c'), '-lm', '-o', executable]);
  TransitionLock.configure(executable);
});
afterAll(async () => { await rm(build, { recursive: true, force: true }); });
beforeEach(async () => { root = await mkdtemp(path.join(os.tmpdir(), 'pm-tr-lock-')); });
afterEach(async () => { for (const lock of leases.splice(0)) await lock.release(); await rm(root, { recursive: true, force: true }); });
const acquire = async (file: string) => { const lock = await TransitionLock.acquire(file); leases.push(lock); return lock; };
it('excludes a second process, heartbeats and releases without changing the inode', async () => {
  const file = path.join(root, 'library.lock');
  const first = await acquire(file), inode = (await stat(file)).ino;
  expect(JSON.parse(await readFile(file, 'utf8'))).toMatchObject({ pid: process.pid, heartbeat: expect.any(Number) });
  await expect(acquire(file)).rejects.toThrow('locked');
  await first.release();
  expect(JSON.parse(await readFile(file, 'utf8')).pid).toBe(0);
  await acquire(file); expect((await stat(file)).ino).toBe(inode);
});
it('requires both a dead owner and an old heartbeat, preserving uncertain records', async () => {
  const file = path.join(root, 'p.pmvlock');
  for (const text of ['', '{bad', JSON.stringify({ pid: process.pid, heartbeat: 0 }), JSON.stringify({ pid: 2147483647, heartbeat: Date.now() })]) {
    await writeFile(file, text); await expect(acquire(file)).rejects.toThrow();
    expect(await readFile(file, 'utf8')).toBe(text);
  }
  await writeFile(file, JSON.stringify({ pid: 2147483647, heartbeat: 0 }));
  await acquire(file);
});
it('honors native flock even when its JSON says released', async () => {
  const file = path.join(root, 'native.pmvlock');
  const child = spawn('python3', ['-c', 'import fcntl,sys; f=open(sys.argv[1],"w"); f.write("{\\"pid\\":0,\\"heartbeat\\":0}"); f.flush(); fcntl.flock(f,fcntl.LOCK_EX); print("ready",flush=True); sys.stdin.read()', file]);
  try {
    await new Promise<void>((resolve, reject) => { child.stdout.once('data', () => resolve()); child.once('exit', () => reject(new Error('fixture exited'))); });
    await expect(acquire(file)).rejects.toThrow('locked');
  } finally { child.stdin.end(); await new Promise(resolve => child.once('exit', resolve)); }
});
it('rejects symlinks and matches native project extension replacement', async () => {
  const target = path.join(root, 'user-data'); await writeFile(target, 'keep');
  const file = path.join(root, 'p.pmvlock'); await symlink(target, file);
  await expect(acquire(file)).rejects.toThrow(); expect(await readFile(target, 'utf8')).toBe('keep');
  expect(projectLockPath('/synthetic/example.pmv')).toBe('/synthetic/example.pmvlock');
});
