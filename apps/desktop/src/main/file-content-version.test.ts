import { createHash } from 'node:crypto';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { afterEach, expect, it, vi } from 'vitest';

const io = vi.hoisted(() => ({ open: vi.fn(), lstat: vi.fn(), read: vi.fn() }));
vi.mock('node:fs/promises', async original => {
  const fs = await original<typeof import('node:fs/promises')>();
  io.read.mockImplementation(read => read());
  io.open.mockImplementation(async (...args) => {
    const handle = await fs.open(...args as Parameters<typeof fs.open>);
    const read = handle.read.bind(handle);
    handle.read = ((...args: Parameters<typeof read>) => io.read(() => read(...args))) as typeof handle.read;
    return handle;
  });
  io.lstat.mockImplementation(fs.lstat);
  return { ...fs, open: io.open, lstat: io.lstat };
});
import { contentIdentity, fileContentVersion, FileContentCache } from './file-content-version';

const roots: string[] = [];
afterEach(async () => {
  vi.clearAllMocks();
  await Promise.all(roots.splice(0).map(root => rm(root, { recursive: true, force: true })));
});
async function fixture() {
  const root = await mkdtemp(path.join(tmpdir(), 'pm-content-version-')); roots.push(root);
  const file = path.join(root, 'synthetic.bin'); await writeFile(file, 'original bytes');
  return file;
}

it('re-hashes after a metadata-only update instead of trusting the first hash', async () => {
  const file = await fixture();
  const realStat = io.lstat.getMockImplementation()!;
  io.lstat.mockImplementationOnce(async (...args) => ({ ...await realStat(...args), ctimeMs: 1 }));
  const result = await fileContentVersion(file);
  expect(result.sha256).toBe(createHash('sha256').update('original bytes').digest('hex'));
  expect(io.read).toHaveBeenCalledTimes(2);
});

it('checks the replacement bytes when a sync client replaces the path during hashing', async () => {
  const file = await fixture();
  io.read.mockImplementationOnce(async read => {
    const result = await read();
    const fs = await vi.importActual<typeof import('node:fs/promises')>('node:fs/promises');
    await writeFile(file + '.incoming', 'replacement bytes');
    await fs.rename(file + '.incoming', file);
    return result;
  });
  expect((await fileContentVersion(file)).sha256).toBe(createHash('sha256').update('replacement bytes').digest('hex'));
  expect(io.read).toHaveBeenCalledTimes(2);
});

it('stops retrying a file that never settles and closes every handle', async () => {
  const file = await fixture();
  const realStat = io.lstat.getMockImplementation()!;
  const realOpen = io.open.getMockImplementation()!;
  const closed: ReturnType<typeof vi.spyOn>[] = [];
  io.lstat.mockImplementation(async (...args) => ({ ...await realStat(...args), ctimeMs: 1 }));
  io.open.mockImplementation(async (...args) => {
    const handle = await realOpen(...args);
    closed.push(vi.spyOn(handle, 'close'));
    return handle;
  });
  try {
    await expect(fileContentVersion(file)).rejects.toThrow('syncing has finished');
    expect(io.read).toHaveBeenCalledTimes(3);
    expect(closed).toHaveLength(3);
    for (const close of closed) expect(close).toHaveBeenCalledTimes(1);
  } finally { io.lstat.mockImplementation(realStat); io.open.mockImplementation(realOpen); }
});

it('rejects a symlink without reading its target', async () => {
  const file = await fixture();
  const { symlink } = await vi.importActual<typeof import('node:fs/promises')>('node:fs/promises');
  await symlink(file, file + '.link');
  await expect(fileContentVersion(file + '.link')).rejects.toThrow();
  expect(io.read).not.toHaveBeenCalled();
  expect(await readFile(file, 'utf8')).toBe('original bytes');
});

it('never accepts an early EOF as a complete cloud file', async () => {
  const file = await fixture();
  io.read.mockResolvedValueOnce({ bytesRead: 0 });
  await expect(fileContentVersion(file)).rejects.toThrow('not fully available');
});

it('reuses a stable checksum and re-verifies metadata changes without changing the content key', async () => {
  const file = await fixture(), cache = new FileContentCache();
  const first = await cache.hash(file);
  expect(await cache.hash(file)).toBe(first);
  expect(io.read).toHaveBeenCalledTimes(1);
  const { utimes } = await vi.importActual<typeof import('node:fs/promises')>('node:fs/promises');
  await utimes(file, new Date('2026-01-01'), new Date('2026-01-01'));
  expect(await cache.hash(file)).toBe(first);
  expect(io.read).toHaveBeenCalledTimes(2);
  await writeFile(file, 'different data');
  expect(await cache.hash(file)).not.toBe(first);
});

it('invalidates a cached checksum after ctime or inode changes', async () => {
  const info = await io.lstat(await fixture());
  expect(contentIdentity(info)).not.toBe(contentIdentity({ ...info, ctimeMs: info.ctimeMs + 1 }));
  expect(contentIdentity(info)).not.toBe(contentIdentity({ ...info, ino: info.ino + 1 }));
});
