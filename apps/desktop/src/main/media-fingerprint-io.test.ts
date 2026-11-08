import { createHash } from 'node:crypto';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { afterEach, expect, it, vi } from 'vitest';

const io = vi.hoisted(() => ({ open: vi.fn(), stat: vi.fn(), read: vi.fn(), limit: Infinity }));
vi.mock('node:fs/promises', async original => {
  const fs = await original<typeof import('node:fs/promises')>();
  io.stat.mockImplementation(fs.stat);
  io.read.mockImplementation(read => read());
  io.open.mockImplementation(async (...args) => {
    const handle = await fs.open(...args as Parameters<typeof fs.open>);
    const read = handle.read.bind(handle);
    handle.read = ((buffer: Buffer, offset: number, length: number, position: number) =>
      io.read(() => read(buffer, offset, Math.min(length, io.limit), position))) as typeof handle.read;
    return handle;
  });
  return { ...fs, open: io.open, stat: io.stat };
});
import { mediaFingerprint } from './media-path-cache';

const roots: string[] = [];
afterEach(async () => {
  io.limit = Infinity; vi.clearAllMocks();
  await Promise.all(roots.splice(0).map(root => rm(root, { recursive: true, force: true })));
});
async function fixture() {
  const root = await mkdtemp(path.join(tmpdir(), 'pm-fingerprint-io-')); roots.push(root);
  const file = path.join(root, 'synthetic.mov'), bytes = Buffer.from('synthetic media bytes');
  await writeFile(file, bytes);
  return { file, expected: `v2:${bytes.length}:${createHash('sha256').update(`powermove-media-v2\n${bytes.length}\n`).update(bytes).digest('hex')}` };
}

it('fills a complete fingerprint sample across partial provider reads', async () => {
  const { file, expected } = await fixture(); io.limit = 3;
  expect(await mediaFingerprint(file)).toBe(expected);
  expect(io.read.mock.calls.length).toBeGreaterThan(1);
});

it('retries a transient cloud read at the same offset', async () => {
  const { file, expected } = await fixture();
  io.read.mockRejectedValueOnce(Object.assign(new Error('provider offline'), { code: 'ENOTCONN' }));
  expect(await mediaFingerprint(file)).toBe(expected);
  expect(io.read).toHaveBeenCalledTimes(2);
});

it('rejects an early EOF instead of recording a partial fingerprint', async () => {
  const { file } = await fixture(); io.limit = 0;
  await expect(mediaFingerprint(file)).rejects.toThrow('not fully available');
});

it('repeats fingerprinting after a provider metadata update during the read', async () => {
  const { file, expected } = await fixture(), original = io.stat.getMockImplementation()!;
  io.stat.mockImplementationOnce(async (...args) => ({ ...await original(...args), ctimeMs: 1 }));
  expect(await mediaFingerprint(file)).toBe(expected);
  expect(io.open).toHaveBeenCalledTimes(2);
});
