import { mkdtemp, readFile, readdir, rm, stat, writeFile, type FileHandle } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { afterEach, expect, it, vi } from 'vitest';

const io = vi.hoisted(() => ({ copySnapshot: vi.fn(), syncFile: vi.fn(), cloneFile: vi.fn(async () => false) }));
vi.mock('@powermove/macos-haptics', () => ({ default: { cloneFile: io.cloneFile, fullSync: async () => false } }));
vi.mock('./durable-file', async importOriginal => {
  const original = await importOriginal<typeof import('./durable-file')>();
  io.copySnapshot.mockImplementation(original.copySnapshot);
  io.syncFile.mockImplementation(original.syncFile);
  return { ...original, copySnapshot: io.copySnapshot, syncFile: io.syncFile };
});
import { FileUpload } from './file-upload';

afterEach(() => { vi.clearAllMocks(); });

async function fixture() {
  const directory = await mkdtemp(path.join(tmpdir(), 'pm-upload-publication-'));
  const destination = path.join(directory, 'result.bin');
  const upload = await FileUpload.create(4, directory);
  const staged = upload as unknown as { directory: string; handle: FileHandle };
  return { directory, destination, upload, staged,
    cleanup: async () => { await upload.dispose(); await rm(directory, { recursive: true, force: true }); } };
}

it('publishes a complete upload once and copies correctly when native cloning is unavailable', async () => {
  const f = await fixture();
  try {
    expect(() => f.upload.claimForExport()).toThrow('incomplete');
    await f.upload.write(new Uint8Array([1, 2, 3, 4]));
    const publish = f.upload.claimForExport();
    expect(() => f.upload.claim()).toThrow('incomplete');
    expect(() => f.upload.claimForExport()).toThrow('incomplete');
    expect(() => f.upload.write(new Uint8Array([1]))).toThrow('Invalid');
    await writeFile(f.destination, 'old target');
    await publish(f.destination);
    expect(await readFile(f.destination)).toEqual(Buffer.from([1, 2, 3, 4]));
    expect(io.copySnapshot).toHaveBeenCalledTimes(1);
    if (process.platform === 'darwin') expect(io.cloneFile).toHaveBeenCalledTimes(1);
    expect(() => publish(f.destination)).toThrow('unavailable');
    await f.upload.dispose();
    expect(await readdir(f.directory)).toEqual(['result.bin']);
  } finally { await f.cleanup(); }
});

it.each(['copy', 'sync'])('preserves an existing target and removes its sibling after %s failure', async failure => {
  const f = await fixture();
  try {
    await f.upload.write(new Uint8Array([1, 2, 3, 4]));
    await writeFile(f.destination, 'old target');
    const publish = f.upload.claimForExport();
    if (failure === 'copy') io.copySnapshot.mockImplementationOnce(async (_source: string, temporary: string) => {
      await writeFile(temporary, 'partial copy', { flag: 'wx' });
      throw new Error('copy failed');
    });
    else io.syncFile.mockRejectedValueOnce(new Error('sync failed'));
    await expect(publish(f.destination)).rejects.toThrow(`${failure} failed`);
    expect(await readFile(f.destination, 'utf8')).toBe('old target');
    expect(() => publish(f.destination)).toThrow('unavailable');
    await f.upload.dispose();
    expect(await readdir(f.directory)).toEqual(['result.bin']);
  } finally { await f.cleanup(); }
});

it('rejects a claimed callback after disposal before publication begins', async () => {
  const f = await fixture();
  try {
    await f.upload.write(new Uint8Array([1, 2, 3, 4]));
    const publish = f.upload.claimForExport();
    await f.upload.dispose();
    expect(() => publish(f.destination)).toThrow('unavailable');
    expect(io.copySnapshot).not.toHaveBeenCalled();
    expect(await readdir(f.directory)).toEqual([]);
  } finally { await f.cleanup(); }
});

it('leaves another writer’s sibling untouched when exclusive copying reports a collision', async () => {
  const f = await fixture();
  let collision = '';
  try {
    await f.upload.write(new Uint8Array([1, 2, 3, 4]));
    await writeFile(f.destination, 'old target');
    io.copySnapshot.mockImplementationOnce(async (_source: string, temporary: string) => {
      collision = temporary;
      await writeFile(temporary, 'other writer', { flag: 'wx' });
      throw Object.assign(new Error('already exists'), { code: 'EEXIST' });
    });
    await expect(f.upload.claimForExport()(f.destination)).rejects.toThrow('already exists');
    expect(await readFile(collision, 'utf8')).toBe('other writer');
    expect(await readFile(f.destination, 'utf8')).toBe('old target');
  } finally { await f.cleanup(); }
});

it('waits for an in-flight publication before disposing its source', async () => {
  const f = await fixture();
  let resume!: () => void;
  const pause = new Promise<void>(resolve => { resume = resolve; });
  let copying = false, disposed = false;
  const originalCopy = io.copySnapshot.getMockImplementation()!;
  io.copySnapshot.mockImplementationOnce(async (...args) => { copying = true; await pause; return originalCopy(...args); });
  let publishing: Promise<void> | undefined, disposing: Promise<void> | undefined;
  try {
    await f.upload.write(new Uint8Array([1, 2, 3, 4]));
    publishing = f.upload.claimForExport()(f.destination);
    await vi.waitFor(() => expect(copying).toBe(true));
    disposing = f.upload.dispose().then(() => { disposed = true; });
    await Promise.resolve(); await Promise.resolve();
    expect(disposed).toBe(false);
    expect(f.staged.handle.fd).not.toBe(-1);
    expect((await stat(path.join(f.staged.directory, 'data'))).size).toBe(4);
    resume(); await publishing; await disposing;
    expect(f.staged.handle.fd).toBe(-1);
    expect(await stat(f.staged.directory).catch(() => null)).toBeNull();
    expect(await readFile(f.destination)).toEqual(Buffer.from([1, 2, 3, 4]));
  } finally { resume(); await publishing; await disposing; await f.cleanup(); }
});
