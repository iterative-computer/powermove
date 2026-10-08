import { mkdtemp, mkdir, readFile, readdir, rm, writeFile, open, stat } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ProjectFiles } from './project-files';
import { decodeProjectContainer, encodeProjectContainer } from '../shared/project-container';
import { readIncrementalIndex, type SaveMedia } from '../shared/project-incremental';
import { nativeHash } from './project-save';
import { readExactly } from './durable-file';
import { build } from 'esbuild';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { fileURLToPath } from 'node:url';

let directory: string, destination: string, registry: string, files: ProjectFiles;
const document = (name: string) => Buffer.from(JSON.stringify({ proj: { name, w: 100, h: 100, layers: [], assets: { clip: { id: 'clip' } } }, history: { entries: [], index: -1 }, ws: { panels: [] } }));
const clip = (revision = 'revision-one', length = 3): SaveMedia => ({ id: 'clip', revision, type: 'video/webm', length });
beforeEach(async () => {
  directory = await mkdtemp(path.join(tmpdir(), 'pm-incremental-'));
  destination = path.join(directory, 'Project.pmv'); registry = path.join(directory, 'registry.json');
  files = new ProjectFiles(registry, path.join(directory, 'backups'));
});
afterEach(async () => { vi.restoreAllMocks(); await rm(directory, { recursive: true, force: true }); });
async function save(name: string, media = [clip()], bytes = Buffer.from([1, 2, 3]), target?: string) {
  const json = document(name), upload = await files.beginIncremental('one', media, json.length, target);
  await upload.write(null, json);
  for (const id of upload.required) {
    for (let offset = 0; offset < bytes.length; offset += 1024 * 1024) await upload.write(id, bytes.subarray(offset, offset + 1024 * 1024));
  }
  await upload.finish(); return upload.required;
}
async function index() {
  const file = await open(destination, 'r');
  try { return await readIncrementalIndex((await file.stat()).size, (offset, length) => readExactly(file, offset, length), nativeHash); }
  finally { await file.close(); }
}

describe('incremental project transactions', () => {
  it('writes a portable file and reuses unchanged media ranges on the next save and after restart', async () => {
    expect(await save('first', [clip()], undefined, destination)).toEqual(['clip']);
    const first = await index(), oldSize = (await stat(destination)).size;
    files = new ProjectFiles(registry, path.join(directory, 'backups'));
    expect(await save('second')).toEqual([]);
    expect((await index()).media).toEqual(first.media);
    expect((await stat(destination)).size - oldSize).toBeLessThan(2000);
    const reopened = decodeProjectContainer(await readFile(destination));
    expect(reopened.document.proj.name).toBe('second');
    expect([...reopened.media[0]!.data]).toEqual([1, 2, 3]);
    expect(decodeProjectContainer(await readFile((await files.backups('one'))[0]!)).document.proj.name).toBe('first');
    const native = await files.open(destination);
    expect(native.document).toEqual(reopened.document);
    expect(native.media[0]!.revision).toBe('revision-one');
    expect([...await files.read(native.token, native.media[0]!.offset, 3)]).toEqual([1, 2, 3]);
    await files.close(native.token);
  });
  it('uploads replacements with the same asset id and size, preserving the old version in backup', async () => {
    await save('first', [clip()], undefined, destination);
    expect(await save('changed', [clip('new')], Buffer.from([4, 5, 6]))).toEqual(['clip']);
    expect([...decodeProjectContainer(await readFile(destination)).media[0]!.data]).toEqual([4, 5, 6]);
    expect([...decodeProjectContainer(await readFile((await files.backups('one'))[0]!)).media[0]!.data]).toEqual([1, 2, 3]);
  });
  it('upgrades PMV3 and legacy JSON without changing the old version in backup', async () => {
    for (const legacy of [document('legacy'), encodeProjectContainer(JSON.parse(document('legacy').toString()), [])]) {
      await files.save('one', legacy, destination);
      await save('upgraded');
      expect((await readFile(destination)).subarray(0, 5).toString()).toBe('PMV4\n');
      expect(await readFile((await files.backups('one'))[0]!)).toEqual(Buffer.from(legacy));
    }
  });
  it('Save As produces a self-contained copy and switches the association', async () => {
    await save('original', [clip()], undefined, destination);
    const target = path.join(directory, 'Copy.pmv');
    expect(await save('copy', [clip()], undefined, target)).toEqual(['clip']);
    expect(await files.destination('one')).toBe(target);
    expect(decodeProjectContainer(await readFile(destination)).document.proj.name).toBe('original');
    expect([...decodeProjectContainer(await readFile(target)).media[0]!.data]).toEqual([1, 2, 3]);
  });
  it('Save As starts a fresh container even when explicitly replacing an unreadable PMV4 file', async () => {
    const damaged = Buffer.from('PMV4\nincomplete old file');
    await writeFile(destination, damaged);
    expect(await save('replacement', [clip()], undefined, destination)).toEqual(['clip']);
    expect(decodeProjectContainer(await readFile(destination)).document.proj.name).toBe('replacement');
    expect(await readFile((await files.backups('one'))[0]!)).toEqual(damaged);
  });
  it('incomplete, cancelled, concurrent, and oversized writes cannot publish or destroy the previous version', async () => {
    await save('first', [clip()], undefined, destination);
    const before = await readFile(destination), json = document('unfinished');
    const upload = await files.beginIncremental('one', [clip('new')], json.length);
    await expect(files.beginIncremental('one', [], json.length)).rejects.toThrow('already in progress');
    expect(() => upload.write('clip', Buffer.from([1]))).toThrow('Invalid');
    expect(() => upload.finish()).toThrow('incomplete');
    expect(() => upload.write(null, Buffer.alloc(1024 * 1024 + 1))).toThrow('Invalid');
    const writing = upload.write(null, json);
    expect(() => upload.write(null, json)).toThrow('already');
    await writing;
    await upload.write('clip', Buffer.from([7]));
    expect(() => upload.finish()).toThrow('incomplete');
    await upload.dispose();
    expect(await readFile(destination)).toEqual(before);
    expect((await readdir(directory)).some(name => name.endsWith('.tmp'))).toBe(false);
    await save('retry');
  });
  it('detects external changes during transfer and blocks stale copies', async () => {
    await save('first', [clip()], undefined, destination);
    const old = await files.open(destination); await files.close(old.token);
    const json = document('editing'), upload = await files.beginIncremental('one', [], json.length);
    await upload.write(null, json);
    await writeFile(destination, 'external work');
    await expect(upload.finish()).rejects.toThrow('changed');
    expect(await readFile(destination, 'utf8')).toBe('external work');
    await expect(files.beginIncremental(old.projectId, [], json.length)).rejects.toThrow('changed');
  });
  it('preserves the destination when backup creation fails', async () => {
    await save('first', [clip()], undefined, destination);
    await mkdir(path.join(directory, 'backups'), { recursive: true });
    await writeFile(path.join(directory, 'backups', 'one'), 'blocked');
    const before = await readFile(destination);
    await expect(save('second')).rejects.toThrow();
    expect(await readFile(destination)).toEqual(before);
  });
  it.skipIf(process.platform !== 'darwin')('preserves the old file on a flush failure and allows a clean retry', async () => {
    const module = await import('@powermove/macos-haptics');
    const native = module.default ?? module;
    await save('first', [clip()], undefined, destination);
    const before = await readFile(destination), json = document('failed');
    const upload = await files.beginIncremental('one', [], json.length);
    await upload.write(null, json);
    const sync = vi.spyOn(native, 'fullSync').mockRejectedValueOnce(new Error('disk full during flush'));
    await expect(upload.finish()).rejects.toThrow('disk full');
    sync.mockRestore();
    expect(await readFile(destination)).toEqual(before);
    expect((await readdir(directory)).some(name => name.endsWith('.tmp'))).toBe(false);
    await save('retry');
  });
  it.skipIf(process.platform !== 'darwin')('restores an external edit that races the final atomic exchange', async () => {
    const module = await import('@powermove/macos-haptics');
    const native = module.default ?? module;
    await save('first', [clip()], undefined, destination);
    const exchange = native.swapFiles;
    const swap = vi.spyOn(native, 'swapFiles').mockImplementationOnce(async (temporary, target) => {
      const swapped = await exchange(temporary, target);
      expect(swapped).toBe(true);
      // A foreign process held the displaced inode open and wrote after swap.
      await writeFile(temporary, 'external work racing commit');
      return swapped;
    });
    await expect(save('second')).rejects.toThrow('changed');
    swap.mockRestore();
    expect(await readFile(destination, 'utf8')).toBe('external work racing commit');
  });
  it.skipIf(process.platform !== 'darwin')('removes an unflushed backup without touching the saved project or backup history', async () => {
    const module = await import('@powermove/macos-haptics'), native = module.default ?? module;
    await save('first', [clip()], undefined, destination);
    await save('second');
    const before = await readFile(destination), backups = await files.backups('one');
    const fullSync = native.fullSync;
    vi.spyOn(native, 'fullSync').mockImplementationOnce(fullSync).mockRejectedValueOnce(new Error('backup flush failed'));
    await expect(save('failed')).rejects.toThrow('backup flush');
    expect(await readFile(destination)).toEqual(before);
    expect(await files.backups('one')).toEqual(backups);
  });
  it.skipIf(process.platform !== 'darwin')('never replaces a file appearing during first-save publication', async () => {
    const module = await import('@powermove/macos-haptics'), native = module.default ?? module;
    const install = native.installFile;
    vi.spyOn(native, 'installFile').mockImplementationOnce(async (temporary, target) => {
      await writeFile(target, 'another project appeared');
      return install(temporary, target);
    });
    await expect(save('first', [clip()], undefined, destination)).rejects.toMatchObject({ code: 'EEXIST' });
    expect(await readFile(destination, 'utf8')).toBe('another project appeared');
    expect(await files.destination('one')).toBeUndefined();
    expect((await readdir(directory)).some(name => name.endsWith('.tmp'))).toBe(false);
  });
  it.skipIf(process.platform !== 'darwin')('failed publication never prunes or fills the successful backup history', async () => {
    const module = await import('@powermove/macos-haptics'), native = module.default ?? module;
    await save('first', [clip()], undefined, destination);
    for (let i = 0; i < 5; i++) await save(`version ${i}`);
    const previousBackups = await files.backups('one');
    const swap = vi.spyOn(native, 'swapFiles').mockRejectedValue(new Error('publication failed'));
    await expect(save('failed')).rejects.toThrow('publication');
    swap.mockRestore();
    expect(await files.backups('one')).toEqual(previousBackups);
    await save('successful retry');
    expect(await files.backups('one')).toHaveLength(5);
  });
  it('reconciles a committed file after association persistence fails', async () => {
    await save('first', [clip()], undefined, destination);
    await rm(registry); await mkdir(registry);
    vi.spyOn(console, 'warn').mockImplementation(() => undefined);
    await save('committed');
    expect(decodeProjectContainer(await readFile(destination)).document.proj.name).toBe('committed');
    expect((await readdir(directory)).some(name => name.includes('.pending-'))).toBe(true);
    await rm(registry, { recursive: true });
    files = new ProjectFiles(registry, path.join(directory, 'backups'));
    expect(await files.destination('one')).toBe(destination);
    await save('after restart');
    expect((await readdir(directory)).some(name => name.includes('.pending-'))).toBe(false);
  });
  it('detects media corruption and truncated commits instead of opening a mixed revision', async () => {
    await save('first', [clip()], undefined, destination);
    const good = await readFile(destination), ranges = await index();
    const corrupt = Buffer.from(good); corrupt[ranges.media[0]!.offset] = corrupt[ranges.media[0]!.offset]! ^ 1;
    await writeFile(destination, corrupt);
    await expect(files.open(destination)).rejects.toThrow('checksum');
    expect(() => decodeProjectContainer(corrupt)).toThrow('checksum');
    await writeFile(destination, good.subarray(0, good.length - 1));
    await expect(files.open(destination)).rejects.toThrow('commit');
  });
  it('does not let an older pending save undo a newer Save As association on restart', async () => {
    await save('first', [clip()], undefined, destination);
    await rm(registry); await mkdir(registry);
    vi.spyOn(console, 'warn').mockImplementation(() => undefined);
    await save('pending');
    await rm(registry, { recursive: true });
    const copy = path.join(directory, 'New destination.pmv');
    await save('newer Save As', [clip()], undefined, copy);
    files = new ProjectFiles(registry, path.join(directory, 'backups'));
    expect(await files.destination('one')).toBe(copy);
  });
  it('reopens the previous complete save after a writer process is killed mid-upload', async () => {
    await save('before crash', [clip()], undefined, destination);
    const before = await readFile(destination);
    const worker = path.join(directory, 'crash-worker.mjs');
    const source = fileURLToPath(new URL('./project-files.ts', import.meta.url));
    const bundle = await build({ stdin: { resolveDir: path.dirname(source), contents: `
      import { ProjectFiles } from ${JSON.stringify(source)};
      const files = new ProjectFiles(process.argv[2], process.argv[3]);
      const save = await files.beginIncremental('one', [], 1000);
      await save.write(null, new Uint8Array([123, 34, 112]));
      console.log('upload-started');
      process.kill(process.pid, 'SIGKILL');
    ` }, bundle: true, platform: 'node', format: 'esm', packages: 'external', write: false });
    await writeFile(worker, bundle.outputFiles[0]!.contents);
    await expect(promisify(execFile)(process.execPath, [worker, registry, path.join(directory, 'backups')])).rejects.toMatchObject({
      stdout: expect.stringContaining('upload-started'),
      ...(process.platform === 'win32' ? { code: 1 } : { signal: 'SIGKILL' })
    });
    expect(await readFile(destination)).toEqual(before);
    files = new ProjectFiles(registry, path.join(directory, 'backups'));
    const reopened = await files.open(destination);
    expect(reopened.document.proj.name).toBe('before crash');
    await files.close(reopened.token);
    await save('after crash');
  });
  it('compacts unreachable media and repeated large document revisions', async () => {
    const payload = Buffer.alloc(17 * 1024 * 1024, 37);
    await save('large', [clip('large', payload.length)], payload, destination);
    const empty = document('removed'), upload = await files.beginIncremental('one', [], empty.length);
    await upload.write(null, empty); await upload.finish();
    expect((await stat(destination)).size).toBeLessThan(2000);
    expect(decodeProjectContainer(await readFile(destination)).media).toEqual([]);
    expect((await stat((await files.backups('one'))[0]!)).size).toBeGreaterThan(payload.length);
  });
  it('handles empty media without requiring an empty IPC chunk', async () => {
    expect(await save('empty', [clip('empty', 0)], Buffer.alloc(0), destination)).toEqual(['clip']);
    expect(decodeProjectContainer(await readFile(destination)).media[0]!.data.length).toBe(0);
    expect(await save('again', [clip('empty', 0)], Buffer.alloc(0))).toEqual([]);
  });
});
