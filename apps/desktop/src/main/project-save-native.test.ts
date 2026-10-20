import { constants, fstatSync } from 'node:fs';
import { chmod, copyFile, link, mkdtemp, readFile, readdir, rename, rm, unlink, utimes, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ProjectFiles } from './project-files';
import { decodeProjectContainer } from '../shared/project-container';
import { temporarySibling } from './durable-file';

const native = vi.hoisted(() => ({ cloneFile: vi.fn(), swapFiles: vi.fn(), installFile: vi.fn(), fullSync: vi.fn() }));
vi.mock('@powermove/macos-haptics', () => ({ default: native }));

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
afterEach(async () => { vi.unstubAllGlobals(); vi.restoreAllMocks(); await rm(directory, { recursive: true, force: true }); });
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
    const conflict = (await readdir(directory)).find(name => name.endsWith('.conflict.pmv'))!;
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
    const conflict = (await readdir(directory)).find(name => name.endsWith('.conflict.pmv'))!;
    expect(await readFile(path.join(directory, conflict), 'utf8')).toBe('external work just before rollback');
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
