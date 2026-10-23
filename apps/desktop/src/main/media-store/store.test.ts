import { createHash } from 'node:crypto';
import { chmod, copyFile, mkdtemp, readFile, rename, rm, stat, utimes, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import { Blake3, blake3Hex } from '../../shared/blake3';
import { FileChanged, hashFile, InProcessExecutor, SEGMENT_BYTES, type HashExecutor } from './hasher';
import type { HashJob, HashJobResult } from './hash-job';
import { MediaStore } from './store';

const executor = new InProcessExecutor();
let root: string;

beforeEach(async () => { root = await mkdtemp(path.join(os.tmpdir(), 'pm-media-store-')); });
afterEach(async () => { await rm(root, { recursive: true, force: true }); });

const bytes = (length: number, seed = 1) => {
  const out = new Uint8Array(length);
  let x = seed;
  for (let i = 0; i < length; i++) { x = (Math.imul(x, 1103515245) + 12345) >>> 0; out[i] = x >>> 24; }
  return out;
};
const old = async (file: string) => { const t = new Date(Date.now() - 60_000); await utimes(file, t, t); };

describe('hashFile', () => {
  it('matches the single-threaded digest across segment boundaries', async () => {
    for (const length of [0, 1, 1024, SEGMENT_BYTES - 1, SEGMENT_BYTES, SEGMENT_BYTES + 1, 2 * SEGMENT_BYTES + 4097]) {
      const data = bytes(length, length + 3);
      const file = path.join(root, `f${length}`);
      await writeFile(file, data);
      await old(file);
      const identity = await hashFile(executor, file);
      expect(identity.hash, `length ${length}`).toBe(blake3Hex(data));
      expect(identity.size).toBe(length);
    }
  });

  it('runs whole segments as subtrees in parallel and still agrees', async () => {
    const data = bytes(3 * SEGMENT_BYTES + 77);
    const file = path.join(root, 'big');
    await writeFile(file, data);
    await old(file);
    const jobs: HashJob['op'][] = [];
    const spy: HashExecutor = { parallelism: 3, close: async () => {}, run: (job, progress) => { jobs.push(job.op); return executor.run(job, progress); } };
    const identity = await hashFile(spy, file);
    expect(jobs).toEqual(['subtree', 'subtree', 'subtree', 'tail']);
    expect(identity.hash).toBe(new Blake3().update(data).hex());
  });

  it('waits for a file that is still being written, then hashes it', async () => {
    const file = path.join(root, 'growing');
    await writeFile(file, bytes(10));
    let now = Date.now();
    let polls = 0;
    const identity = await hashFile(executor, file, {
      settleMs: 1000, now: () => now,
      sleep: async (ms) => { polls++; if (polls === 2) await writeFile(file, bytes(20)); now += ms; }
    });
    expect(identity.size).toBe(20);
    expect(polls).toBeGreaterThan(4);
  });

  it('hashes again when the file changes during hashing, and gives up after the attempts', async () => {
    const file = path.join(root, 'moving');
    await writeFile(file, bytes(100));
    await old(file);
    let runs = 0;
    const changing: HashExecutor = {
      parallelism: 1, close: async () => {},
      run: async (job, progress): Promise<HashJobResult> => {
        runs++;
        const result = await executor.run(job, progress);
        if (runs === 1) { await writeFile(file, bytes(100, 9)); await old(file); const t = new Date(Date.now() - 30_000); await utimes(file, t, t); }
        return result;
      }
    };
    const identity = await hashFile(changing, file);
    expect(runs).toBe(2);
    expect(identity.hash).toBe(blake3Hex(bytes(100, 9)));

    let count = 0;
    const always: HashExecutor = {
      parallelism: 1, close: async () => {},
      run: async (job, progress) => { const result = await executor.run(job, progress); const t = new Date(Date.now() - 50_000 + ++count * 1000); await utimes(file, t, t); return result; }
    };
    await expect(hashFile(always, file, { attempts: 2 })).rejects.toBeInstanceOf(FileChanged);
  });
});

describe('MediaStore', () => {
  it('copies a file under its hash, records the key, and deduplicates', async () => {
    const store = new MediaStore({ root: path.join(root, 'store'), executor, now: () => 42 });
    const source = path.join(root, 'clip.mov');
    const data = bytes(70_000);
    await writeFile(source, data);
    const record = await store.putFile(source, { key: 'media:v2:70000:abc', type: 'video/quicktime' });
    expect(record).toMatchObject({ key: 'media:v2:70000:abc', hash: blake3Hex(data), size: 70_000, type: 'video/quicktime', revision: blake3Hex(data), at: 42, source: 'import' });
    const object = store.objectPath(record.hash);
    expect(new Uint8Array(await readFile(object))).toEqual(data);
    expect((await stat(object)).mode & 0o777).toBe(0o400);
    expect(await store.record('media:v2:70000:abc')).toMatchObject({ hash: record.hash });
    // The original is untouched.
    expect(new Uint8Array(await readFile(source))).toEqual(data);
    // Importing an immutable object under another key must leave it read-only.
    const again = await store.putFile(object, { key: 'other' });
    expect((await stat(object)).mode & 0o777).toBe(0o400);
    expect(again.hash).toBe(record.hash);
    expect((await store.records()).map((r) => r.key).sort()).toEqual(['media:v2:70000:abc', 'other']);
  });

  it('forgets a key without deleting its object', async () => {
    const store = new MediaStore({ root: path.join(root, 'store'), executor });
    const source = path.join(root, 'a.png');
    await writeFile(source, bytes(500));
    const record = await store.putFile(source, { key: 'k1' });
    expect(await store.removeKey('k1')).toBe(true);
    expect(await store.record('k1')).toBeNull();
    expect(await store.hasObject(record.hash, 500)).toBe(true);
  });

  it('commits a streamed upload only when the bytes on disk hash to the sender hash', async () => {
    const store = new MediaStore({ root: path.join(root, 'store'), executor });
    const data = bytes(9 * 1024 * 1024 + 5);
    const token = await store.beginStream(7, { key: 'media:x', type: 'image/png', size: data.length, revision: 'rev-1', origin: 'indexeddb' });
    for (let offset = 0; offset < data.length; offset += 4 * 1024 * 1024) await store.chunk(7, token, offset, data.subarray(offset, offset + 4 * 1024 * 1024));
    const record = await store.finishStream(7, token, { blake3: blake3Hex(data) });
    expect(record).toMatchObject({ key: 'media:x', hash: blake3Hex(data), size: data.length, revision: 'rev-1', source: 'indexeddb' });
    expect(Buffer.compare(await readFile(store.objectPath(record.hash)), data)).toBe(0);

    const bad = await store.beginStream(7, { key: 'media:y', type: '', size: 10 });
    await store.chunk(7, bad, 0, bytes(10));
    await expect(store.finishStream(7, bad, { blake3: blake3Hex(bytes(10, 2)) })).rejects.toThrow(/does not match/);
    expect(await store.record('media:y')).toBeNull();
    expect(await store.hasObject(blake3Hex(bytes(10)))).toBe(false);
  });

  it('commits bytes main read from a project file against its recorded SHA-256', async () => {
    const store = new MediaStore({ root: path.join(root, 'store'), executor });
    const data = bytes(5 * 1024 * 1024 + 9, 4);
    const sha256 = createHash('sha256').update(data).digest('hex');
    const token = await store.beginStream(3, { key: 'media:p', type: 'video/mp4', size: data.length, revision: 'r1', origin: 'project' });
    for (let offset = 0; offset < data.length; offset += 1024 * 1024) await store.chunk(3, token, offset, data.subarray(offset, offset + 1024 * 1024), 'main');
    const record = await store.finishStream(3, token, { sha256 });
    expect(record).toMatchObject({ key: 'media:p', hash: blake3Hex(data), size: data.length, revision: 'r1', source: 'project', sha256 });
    expect(Buffer.compare(await readFile(store.objectPath(record.hash)), data)).toBe(0);

    // A range that is not what the file recorded is never committed.
    const wrong = await store.beginStream(3, { key: 'media:q', type: '', size: 16, origin: 'project' });
    await store.chunk(3, wrong, 0, bytes(16, 8), 'main');
    await expect(store.finishStream(3, wrong, { sha256 })).rejects.toThrow(/checksum/);
    expect(await store.record('media:q')).toBeNull();
    expect(await store.hasObject(blake3Hex(bytes(16, 8)))).toBe(false);
  });

  it('accepts a project range without a recorded checksum only when main read every byte', async () => {
    const store = new MediaStore({ root: path.join(root, 'store'), executor });
    const data = bytes(3000, 6);
    const mainRead = await store.beginStream(3, { key: 'media:m', type: '', size: data.length, origin: 'project' });
    await store.chunk(3, mainRead, 0, data, 'main');
    expect(await store.finishStream(3, mainRead, {})).toMatchObject({ key: 'media:m', hash: blake3Hex(data) });
    // Bytes from the page always name their BLAKE3 (or a checksum).
    const fromPage = await store.beginStream(3, { key: 'media:n', type: '', size: data.length });
    await store.chunk(3, fromPage, 0, data);
    await expect(store.finishStream(3, fromPage, {})).rejects.toThrow(/BLAKE3/);
    expect(await store.record('media:n')).toBeNull();
    // Mixed: one page chunk is enough to need a hash.
    const mixed = await store.beginStream(3, { key: 'media:o', type: '', size: data.length });
    await store.chunk(3, mixed, 0, data.subarray(0, 1000), 'main');
    await store.chunk(3, mixed, 1000, data.subarray(1000));
    await expect(store.finishStream(3, mixed, {})).rejects.toThrow(/BLAKE3/);
    // An empty upload has no bytes main read, so it names its hash too.
    const empty = await store.beginStream(3, { key: 'media:e', type: '', size: 0 });
    await expect(store.finishStream(3, empty, {})).rejects.toThrow(/BLAKE3/);
    expect(await store.record('media:e')).toBeNull();
    const named = await store.beginStream(3, { key: 'media:e', type: '', size: 0 });
    expect(await store.finishStream(3, named, { blake3: blake3Hex(new Uint8Array(0)) })).toMatchObject({ key: 'media:e', size: 0 });
  });

  it('refuses out-of-order chunks, other owners, and incomplete uploads', async () => {
    const store = new MediaStore({ root: path.join(root, 'store'), executor });
    const token = await store.beginStream(1, { key: 'k', type: '', size: 8 });
    await expect(store.chunk(2, token, 0, bytes(4))).rejects.toThrow(/Unknown/);
    await expect(store.chunk(1, token, 4, bytes(4))).rejects.toThrow(/Invalid/);
    await store.chunk(1, token, 0, bytes(4));
    await expect(store.finishStream(1, token, { blake3: blake3Hex(bytes(4)) })).rejects.toThrow(/incomplete/);
    await expect(store.beginStream(1, { key: '../../etc', type: '', size: 1 })).rejects.toThrow(/key/);
  });

  it('sweeps partial writes left by a crash', async () => {
    const storeRoot = path.join(root, 'store');
    const first = new MediaStore({ root: storeRoot, executor });
    await first.beginStream(1, { key: 'k', type: '', size: 4 });
    const second = new MediaStore({ root: storeRoot, executor });
    await second.init();
    const { readdir } = await import('node:fs/promises');
    expect(await readdir(path.join(storeRoot, 'tmp'))).toEqual([]);
  });

  it('verifies a link by size and time, rehashes when the time moved, and reports changes', async () => {
    const store = new MediaStore({ root: path.join(root, 'store'), executor });
    const source = path.join(root, 'linked.wav');
    await writeFile(source, bytes(4000));
    await old(source);
    const identity = await store.identify(source);
    const link = { path: source, hash: identity.hash, size: identity.size, mtimeMs: identity.mtimeMs };
    expect(await store.verifyLink(link)).toMatchObject({ status: 'ok', identity: null });
    const t = new Date(Date.now() - 10_000); await utimes(source, t, t);
    expect(await store.verifyLink(link)).toMatchObject({ status: 'ok', identity: { hash: identity.hash } });
    await chmod(source, 0o600);
    await writeFile(source, bytes(4000, 5));
    const later = new Date(Date.now() - 5_000); await utimes(source, later, later);
    expect(await store.verifyLink(link)).toEqual({ status: 'changed' });
    await rm(source);
    expect(await store.verifyLink(link)).toEqual({ status: 'missing' });
  });

  it('hashes again when another file with the same size and dates replaced the link', async () => {
    const store = new MediaStore({ root: path.join(root, 'store'), executor });
    const source = path.join(root, 'linked.mov');
    await writeFile(source, bytes(4000, 1));
    await old(source);
    const identity = await store.identify(source);
    const link = { path: source, hash: identity.hash, size: identity.size, mtimeMs: identity.mtimeMs, dev: identity.dev, ino: identity.ino };
    // Another file, its dates set to the original's (cp -p, a sync tool).
    const other = path.join(root, 'other.mov');
    await writeFile(other, bytes(4000, 2));
    const when = new Date(identity.mtimeMs);
    await utimes(other, when, when);
    await rename(other, source);
    expect((await stat(source)).mtimeMs).toBe(identity.mtimeMs);
    expect(await store.verifyLink(link)).toEqual({ status: 'changed' });
    // The original content back under a new file number is still the original.
    const copy = path.join(root, 'copy.mov');
    await writeFile(copy, bytes(4000, 1));
    await utimes(copy, when, when);
    await copyFile(copy, source);
    await utimes(source, when, when);
    expect(await store.verifyLink(link)).toMatchObject({ status: 'ok', identity: { hash: identity.hash } });
  });
});
