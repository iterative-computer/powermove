import { EventEmitter } from 'node:events';
import { chmod, mkdir, mkdtemp, readFile, rm, type FileHandle } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { afterEach, expect, it, vi } from 'vitest';
import type { IpcMain, IpcMainInvokeEvent } from 'electron';
vi.mock('electron', () => ({ BrowserWindow: { fromWebContents: () => ({ isDestroyed: () => false }) }, dialog: {} }));
import { registerSaveIpc } from './save';
import { FileUpload } from './file-upload';
import { IPC } from '../shared/ipc';

afterEach(() => { vi.restoreAllMocks(); });

it.each([false, true])('preserves the actual write error when spool cleanup also fails, successful delivery=%s', async delivered => {
  const folder = await mkdtemp(path.join(tmpdir(), 'powermove-upload-cleanup-error-'));
  const destination = path.join(folder, delivered ? 'saved.bin' : 'destination-directory');
  if (!delivered) await mkdir(destination);
  const sender = Object.assign(new EventEmitter(), { id: 1, isDestroyed: () => false, isCrashed: () => false });
  const handlers = new Map<string, (event: IpcMainInvokeEvent, input: unknown) => any>();
  registerSaveIpc({ handle: (name: string, handler: any) => handlers.set(name, handler) } as Pick<IpcMain, 'handle'>,
    { isTrustedSender: () => true, dialogs: { showSave: async () => ({ canceled: false, filePath: destination }) } });
  const call = (name: string, input: unknown) => handlers.get(name)!({ sender } as unknown as IpcMainInvokeEvent, input);
  const original = FileUpload.create;
  let spool!: FileUpload;
  vi.spyOn(FileUpload, 'create').mockImplementation(async size => { spool = await original(size, folder); return spool; });
  const token = await call(IPC.fileSaveUpload, 3);
  await call(IPC.fileSaveChunk, { uploadId: token, data: new Uint8Array([1, 2, 3]) });
  const staged = spool as unknown as { handle: FileHandle; directory: string };
  try {
    // Reading still succeeds through the open handle. Cleanup cannot unlink
    // its private data file until the directory becomes writable again.
    await chmod(staged.directory, 0o500);
    const outcome = await call(IPC.fileSave, { name: path.basename(destination), uploadId: token })
      .then((value: unknown) => ({ value }), (error: Error) => ({ error }));
    if (delivered) {
      expect(outcome).toMatchObject({ error: expect.any(Error) });
      expect(['EACCES', 'EPERM']).toContain((outcome.error as NodeJS.ErrnoException).code);
      expect(await readFile(destination)).toEqual(Buffer.from([1, 2, 3]));
    } else {
      expect(outcome).toEqual({ value: { ok: false, cancelled: false, error: expect.stringContaining('EISDIR') } });
    }
    expect(staged.handle.fd).toBe(-1);
  } finally {
    await chmod(staged.directory, 0o700).catch(() => undefined);
    await call(IPC.fileSaveAbort, token);
    sender.emit('destroyed');
    await rm(folder, { recursive: true, force: true });
  }
});
