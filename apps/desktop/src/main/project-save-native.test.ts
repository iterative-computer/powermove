import { constants, fstatSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { chmod, copyFile, link, mkdtemp, readFile, readdir, rename, rm, stat, unlink, utimes, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ProjectFiles } from './project-files';
import { decodeProjectContainer } from '../shared/project-container';
import { temporarySibling } from './durable-file';

const native = vi.hoisted(() => ({ cloneFile: vi.fn(), swapFiles: vi.fn(), installFile: vi.fn(), fullSync: vi.fn() }));
vi.mock('@powermove/macos-haptics', () => ({ default: native }));
// Lets a test run a side effect in the middle of a file being hashed.
const reads = vi.hoisted(() => ({ before: undefined as undefined | (() => Promise<void>) }));
vi.mock('./read-retry', async importOriginal => {
  const actual = await importOriginal<typeof import('./read-retry')>();
  return { ...actual, readWithRetry: async <T>(read: () => Promise<T>) => {
    const hook = reads.before; reads.before = undefined;
    await hook?.();
    return actual.readWithRetry(read);
  } };
});

// Exercise the native protocol on every CI host with real temporary files.
// This models exchange checkpoints; it does NOT validate APFS syscall behavior.
const hostProcess = process;
let directory: string, destination: string, registry: string, files: ProjectFiles;
const document = (name: string) => Buffer.from(JSON.stringify({ proj: { name, w: 100, h: 100, layers: [] } }));
async function exchange(source: string, target: string) {
  const holding = temporarySibling(target);
  await rename(source, holding); await rename(target, source); await rename(holding, target);
  return true;
}
beforeEach(async () => {
  vi.stubGlobal('process', new Proxy(hostProcess, { get: (target, key) => key === 'platform' ? 'darwin' : Reflect.get(target, key) }));
  native.cloneFile.mockReset().mockImplementation(async (source, target) => { await copyFile(source, target, constants.COPYFILE_EXCL); return true; });
  native.swapFiles.mockReset().mockImplementation(exchange);
  native.installFile.mockReset().mockImplementation(async (source, target) => { await link(source, target); await unlink(source); return true; });
  native.fullSync.mockReset().mockResolvedValue(false);
  directory = await mkdtemp(path.join(tmpdir(), 'pm-native-save-'));
  destination = path.join(directory, 'Project.pmv'); registry = path.join(directory, 'registry.json');
  files = new ProjectFiles(registry, path.join(directory, 'backups'));
});
afterEach(async () => { reads.before = undefined; vi.unstubAllGlobals(); vi.restoreAllMocks(); await rm(directory, { recursive: true, force: true }); });
async function save(name: string, target?: string) {
  const json = document(name), upload = await files.beginIncremental('one', [], json.length, target);
  await upload.write(null, json); return upload.finish();
}

describe('native save publication protocol', () => {
  it('repeatedly clones, exchanges and refreshes the saved hash, including after restart', async () => {
    await save('first', destination);
    for (let i = 0; i < 4; i++) await save(`revision ${i}`);
    files = new ProjectFiles(registry, path.join(directory, 'backups'));
    await save('after restart');
    expect(decodeProjectContainer(await readFile(destination)).document.proj.name).toBe('after restart');
    expect(native.swapFiles).toHaveBeenCalledTimes(5);
    expect(await files.backups('one')).toHaveLength(5);
  });
  it('verifies bytes when clone/backup preparation changes only source metadata', async () => {
    await save('first', destination);
    native.cloneFile.mockImplementation(async (source, target) => {
      await copyFile(source, target, constants.COPYFILE_EXCL);
      await utimes(source, new Date(1000), new Date(2000));
      return true;
    });
    await save('second');
    expect(decodeProjectContainer(await readFile(destination)).document.proj.name).toBe('second');
    expect(decodeProjectContainer(await readFile((await files.backups('one'))[0]!)).document.proj.name).toBe('first');
  });
  it('rejects a different staged prefix even when the source is unchanged by the time it is checked', async () => {
    await save('first', destination);
    const before = await readFile(destination);
    native.cloneFile.mockImplementationOnce(async (source, target) => {
      await copyFile(source, target, constants.COPYFILE_EXCL);
      const corrupt = Buffer.from(before); corrupt[5] = corrupt[5]! ^ 1;
      await writeFile(target, corrupt);
      return true;
    });
    await expect(save('second')).rejects.toThrow('changed');
    expect(await readFile(destination)).toEqual(before);
    expect(native.swapFiles).not.toHaveBeenCalled();
    await save('retry');
  });
  it('restores an external edit at exchange without advancing the association', async () => {
    await save('first', destination);
    native.swapFiles.mockImplementationOnce(async (source, target) => {
      await writeFile(target, 'external work');
      return exchange(source, target);
    });
    await expect(save('second')).rejects.toThrow('changed');
    expect(await readFile(destination, 'utf8')).toBe('external work');
    await expect(save('third')).rejects.toThrow('changed');
  });
  it('keeps both external versions when the published file changes before conflict handling', async () => {
    await save('first', destination);
    native.swapFiles.mockImplementationOnce(async (source, target) => {
      await writeFile(target, 'external work before exchange');
      await exchange(source, target);
      await writeFile(target, 'external work after exchange');
      return true;
    });
    await expect(save('second')).rejects.toThrow('preserved at');
    expect(await readFile(destination, 'utf8')).toBe('external work after exchange');
    const conflict = (await readdir(directory)).find(name => /^Project \(other version \d{4}-\d\d-\d\d \d\d\.\d\d\.\d\d\)\.pmv$/.test(name))!;
    expect(await readFile(path.join(directory, conflict), 'utf8')).toBe('external work before exchange');
  });
  it('preserves a writer racing the rollback itself instead of deleting its bytes in dispose', async () => {
    await save('first', destination);
    native.swapFiles.mockImplementationOnce(async (source, target) => {
      await writeFile(target, 'external work before exchange');
      return exchange(source, target);
    }).mockImplementationOnce(async (source, target) => {
      await writeFile(target, 'external work just before rollback');
      return exchange(source, target);
    });
    await expect(save('second')).rejects.toThrow('preserved at');
    expect(await readFile(destination, 'utf8')).toBe('external work before exchange');
    const conflict = (await readdir(directory)).find(name => /^Project \(other version \d{4}-\d\d-\d\d \d\d\.\d\d\.\d\d\)\.pmv$/.test(name))!;
    expect(await readFile(path.join(directory, conflict), 'utf8')).toBe('external work just before rollback');
  });
  it('saves repeatedly when a cloud provider touches the displaced file\'s metadata after every exchange', async () => {
    await save('first', destination);
    // iCloud Drive's file provider rewrites xattrs on the displaced inode
    // ~45-160 ms after each exchange: ctime moves, bytes do not.
    native.swapFiles.mockImplementation(async (source, target) => {
      await exchange(source, target);
      // Lands while the displaced file is being hashed for verification.
      const mode = (await stat(source)).mode & 0o777;
      reads.before = async () => { await new Promise(resolve => setTimeout(resolve, 5)); await chmod(source, mode); };
      return true;
    });
    for (let i = 0; i < 5; i++) await save(`cloud revision ${i}`);
    expect(decodeProjectContainer(await readFile(destination)).document.proj.name).toBe('cloud revision 4');
    expect((await readdir(directory)).filter(name => name !== 'Project.pmv' && name !== 'registry.json' && name !== 'backups')).toEqual([]);
  });
  it('accepts its own interrupted save instead of blocking every later save', async () => {
    await save('first', destination);
    let displaced = '';
    native.swapFiles.mockImplementationOnce(async (source, target) => {
      await exchange(source, target);
      displaced = source;
      // The previous version cannot be verified (e.g. a sync client holds it).
      await chmod(source, 0o000);
      return true;
    });
    const failure = await save('second').catch(error => error);
    await chmod(path.join(directory, (await readdir(directory)).find(name => name.includes('(other version'))!), 0o600);
    expect(failure.message).toContain('Could not verify the previous version');
    expect(failure.recoveryPath).not.toBe(displaced);
    expect(decodeProjectContainer(await readFile(destination)).document.proj.name).toBe('second');
    // Later saves work in this session and after restart.
    await save('third');
    files = new ProjectFiles(registry, path.join(directory, 'backups'));
    await save('after restart');
    expect(decodeProjectContainer(await readFile(destination)).document.proj.name).toBe('after restart');
    await writeFile(destination, 'external');
    await expect(save('must not overwrite')).rejects.toThrow('changed');
  });
  it('resumes saving when a pending save note proves the bytes on disk are its own', async () => {
    await save('first', destination);
    const registryValue = JSON.parse(await readFile(registry, 'utf8'));
    // Model the field failure: our publish landed, but the registry kept the
    // previous hash and only the pending journal records the new one.
    await save('second');
    const landed = createHash('sha256').update(await readFile(destination)).digest('hex');
    await writeFile(registry, JSON.stringify(registryValue));
    files = new ProjectFiles(registry, path.join(directory, 'backups'));
    await files.destination('one');
    await expect(save('blocked')).rejects.toThrow('changed');
    // The note appears while the app is running (not only at startup).
    await writeFile(registry + '.pending-test.json', JSON.stringify({ ...registryValue.one, id: 'one', hash: landed, generation: 99 }));
    await save('third');
    expect(decodeProjectContainer(await readFile(destination)).document.proj.name).toBe('third');
  });
  it('still refuses a different file even with an unrelated pending save note', async () => {
    await save('first', destination);
    await writeFile(destination, 'external');
    const registryValue = JSON.parse(await readFile(registry, 'utf8'));
    await writeFile(registry + '.pending-test.json', JSON.stringify({ ...registryValue.one, id: 'one', hash: 'f'.repeat(64), generation: 99 }));
    await expect(save('second')).rejects.toThrow('changed');
    expect(await readFile(destination, 'utf8')).toBe('external');
  });
  it.skipIf(process.platform === 'win32' || process.getuid?.() === 0).each(['incremental', 'legacy'])('retains displaced work even if its recovery rename fails (%s)', async format => {
    await save('first', destination);
    let displacedPath!: string;
    native.swapFiles.mockImplementationOnce(async (source, target) => {
      await writeFile(target, 'external before exchange');
      await exchange(source, target);
      await writeFile(target, 'external after exchange');
      displacedPath = source;
      await chmod(directory, 0o500);
      return true;
    });
    try {
      const failure = await (format === 'incremental' ? save('second') : files.save('one', document('second'))).catch(error => error);
      expect(failure).toMatchObject({ recoveryPath: displacedPath });
      expect(failure.message).toContain(`preserved at ${displacedPath}`);
      expect(await readFile(destination, 'utf8')).toBe('external after exchange');
      expect(await readFile(displacedPath, 'utf8')).toBe('external before exchange');
    } finally { await chmod(directory, 0o700); }
  });
  it('reports a post-publication flush failure, permits retry, and still rejects subsequent external changes', async () => {
    await save('first', destination);
    native.fullSync.mockImplementation(async fd => {
      // Inspect the actual descriptor without taking ownership of it.
      if (fstatSync(fd).isDirectory() && decodeProjectContainer(await readFile(destination)).document.proj.name === 'second') {
        throw new Error('directory flush failed');
      }
      return false;
    });
    await expect(save('second')).rejects.toThrow('file was written');
    expect(decodeProjectContainer(await readFile(destination)).document.proj.name).toBe('second');
    native.fullSync.mockResolvedValue(false);
    files = new ProjectFiles(registry, path.join(directory, 'backups'));
    await save('after restart');
    await writeFile(destination, 'external after failed flush');
    await expect(save('must not overwrite')).rejects.toThrow('changed');
    expect(await readFile(destination, 'utf8')).toBe('external after failed flush');
  });
  it('keeps a first-save association accurate after publication finalization fails', async () => {
    native.fullSync.mockImplementation(async fd => {
      if (fstatSync(fd).isDirectory() && await readFile(destination).then(() => true, () => false)) throw new Error('directory flush failed');
      return false;
    });
    await expect(save('first', destination)).rejects.toThrow('file was written');
    expect(await files.destination('one')).toBe(destination);
    native.fullSync.mockResolvedValue(false);
    await save('retry');
    expect(decodeProjectContainer(await readFile(destination)).document.proj.name).toBe('retry');
  });
  it('never overwrites a file appearing during exclusive first-save publication', async () => {
    const install = native.installFile.getMockImplementation()!;
    native.installFile.mockImplementationOnce(async (source, target) => {
      await writeFile(target, 'another project appeared');
      return install(source, target);
    });
    await expect(save('first', destination)).rejects.toMatchObject({ code: 'EEXIST' });
    expect(await readFile(destination, 'utf8')).toBe('another project appeared');
    expect(await files.destination('one')).toBeUndefined();
  });
});
