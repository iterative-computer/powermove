import { describe, expect, test } from 'bun:test';
import { mkdtemp, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { REGISTRY_LIMITS } from '../src/limits';
import { snapshot, type SnapshotInput } from '../src/snapshot';
import { gunzip, gzip, readTar, readTarGz, writeTar, writeTarGz } from '../src/tar';

const enc = new TextEncoder();
const file = (path: string, text: string): SnapshotInput => ({ path, bytes: enc.encode(text) });

function patchedHeader(tar: Uint8Array, change: (header: Uint8Array) => void): Uint8Array {
  const copy = tar.slice();
  const header = copy.subarray(0, 512);
  change(header);
  header.fill(32, 148, 156);
  let sum = 0;
  for (const byte of header) sum += byte;
  header.set(enc.encode(sum.toString(8).padStart(6, '0') + '\0 '), 148);
  return copy;
}

describe('tar', () => {
  test('deterministic round trip preserves snapshot', async () => {
    const files = [file('z.txt', 'last'), file('nested/a.txt', 'first'), file('empty', '')];
    const tar = writeTar(files);
    expect(tar.length % 512).toBe(0);
    expect(tar.slice(-1024).every((byte) => byte === 0)).toBe(true);
    const read = readTar(tar);
    expect(read.map((entry) => entry.path)).toEqual(['empty', 'nested/a.txt', 'z.txt']);
    expect((await snapshot(read)).treeSha).toBe((await snapshot(files)).treeSha);
    expect((await snapshot(await readTarGz(await writeTarGz(files)))).treeSha).toBe((await snapshot(files)).treeSha);
    expect(await gunzip(await gzip(tar), tar.length)).toEqual(tar);
    await expect(gunzip(await gzip(tar), 10)).rejects.toMatchObject({ code: 'tree_too_large' });
  });

  test('checksum and unsafe entry rejection', () => {
    const tar = writeTar([file('safe', 'abc')]);
    const corrupt = tar.slice(); corrupt[0] = 88;
    expect(() => readTar(corrupt)).toThrow();
    for (const path of ['../bad', '/absolute', 'a/../bad']) {
      const unsafe = patchedHeader(tar, (header) => { header.fill(0, 0, 100); header.set(enc.encode(path)); });
      expect(() => readTar(unsafe)).toThrow();
    }
    const symlink = patchedHeader(tar, (header) => { header[156] = 50; });
    expect(() => readTar(symlink)).toThrow();
    try { readTar(symlink); } catch (error) { expect(error).toMatchObject({ code: 'unsupported_entry' }); }
  });

  test('caps and truncation', () => {
    const tar = writeTar([file('a', 'abc'), file('b', 'xyz')]);
    expect(() => readTar(tar, { ...REGISTRY_LIMITS, files: 1 })).toThrow();
    expect(() => readTar(tar, { ...REGISTRY_LIMITS, fileBytes: 2 })).toThrow();
    expect(() => readTar(tar, { ...REGISTRY_LIMITS, treeBytes: 5 })).toThrow();
    expect(() => readTar(tar.subarray(0, 600))).toThrow();
    expect(() => readTar(tar.subarray(0, 512))).toThrow();
  });

  test('ustar prefix and unsplittable names', () => {
    const path = `${'p'.repeat(110)}/leaf.txt`;
    expect(readTar(writeTar([file(path, 'ok')]))[0]?.path).toBe(path);
    expect(() => writeTar([file('x'.repeat(101), '')])).toThrow();
  });

  test('accepts plain v7 regular-file headers', () => {
    const tar = patchedHeader(writeTar([file('legacy.txt', 'v7')]), (header) => {
      header.fill(0, 257, 265);
    });
    expect(readTar(tar)).toEqual([file('legacy.txt', 'v7')]);
  });

  test.skipIf(!Bun.which('tar'))('system tar lists the same names in bytewise order', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'registry-tar-'));
    const archive = join(dir, 'test.tar');
    try {
      await writeFile(archive, writeTar([file('b', ''), file('a/x', ''), file('a-b', '')]));
      const process = Bun.spawn(['tar', '-tf', archive], { stdout: 'pipe', stderr: 'pipe' });
      const [out, status] = await Promise.all([new Response(process.stdout).text(), process.exited]);
      expect(status).toBe(0);
      expect(out.trim().split('\n')).toEqual(['a-b', 'a/x', 'b']);
    } finally { await rm(dir, { recursive: true, force: true }); }
  });
});
