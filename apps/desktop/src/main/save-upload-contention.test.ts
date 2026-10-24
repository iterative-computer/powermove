import { EventEmitter } from 'node:events';
import { mkdtemp, readFile, rm, stat, type FileHandle } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { afterEach, expect, it, vi } from 'vitest';
import type { IpcMain, IpcMainInvokeEvent } from 'electron';
vi.mock('electron', () => ({ BrowserWindow: { fromWebContents: () => ({ isDestroyed: () => false }) }, dialog: {} }));
import { registerSaveIpc } from './save';
import { FileUpload } from './file-upload';
import { IPC } from '../shared/ipc';

afterEach(() => { vi.useRealTimers(); vi.restoreAllMocks(); });

async function fixture() {
  const folder = await mkdtemp(path.join(tmpdir(), 'powermove-save-upload-contention-'));
  const destination = path.join(folder, 'result.bin');
  const sender = Object.assign(new EventEmitter(), { id: 1, isDestroyed: () => false, isCrashed: () => false });
  const handlers = new Map<string, (event: IpcMainInvokeEvent, input: unknown) => any>();
  let finishChoice!: () => void;
  let choosing = false;
  const showSave = vi.fn(async () => ({ canceled: false, filePath: destination }));
  showSave.mockImplementationOnce(() => new Promise(resolve => {
    choosing = true; finishChoice = () => resolve({ canceled: true, filePath: '' });
  }));
  registerSaveIpc({ handle: (name: string, handler: any) => handlers.set(name, handler) } as Pick<IpcMain, 'handle'>,
    { isTrustedSender: () => true, dialogs: { showSave } });
  const call = (name: string, input: unknown) => handlers.get(name)!({ sender } as unknown as IpcMainInvokeEvent, input);
  const original = FileUpload.create;
  let spool!: FileUpload;
  vi.spyOn(FileUpload, 'create').mockImplementation(async size => { spool = await original(size, folder); return spool; });
  const token = await call(IPC.fileSaveUpload, 3);
  await call(IPC.fileSaveChunk, { uploadId: token, data: new Uint8Array([1, 2, 3]) });
  const staged = spool as unknown as { handle: FileHandle; directory: string };
  const choice = call(IPC.exportChoose, { name: 'concurrent.png' });
  await vi.waitFor(() => expect(choosing).toBe(true));
  return { folder, destination, sender, call, token, staged, choice, finishChoice };
}

it('leaves a completed upload retryable when a pending destination choice rejects its save', async () => {
  const f = await fixture();
  try {
    expect(await f.call(IPC.fileSave, { name: 'result.bin', uploadId: f.token })).toMatchObject({ ok: false, error: 'A save is already in progress.' });
    f.finishChoice(); await f.choice;
    expect(await f.call(IPC.fileSave, { name: 'result.bin', uploadId: f.token })).toEqual({ ok: true, path: f.destination });
    expect(await readFile(f.destination)).toEqual(Buffer.from([1, 2, 3]));
    expect(f.staged.handle.fd).toBe(-1);
    expect(await stat(f.staged.directory).catch(() => null)).toBeNull();
  } finally {
    f.finishChoice(); await f.choice;
    await f.call(IPC.fileSaveAbort, f.token);
    f.sender.emit('destroyed');
    await rm(f.folder, { recursive: true, force: true });
  }
});

it('still expires and removes the real idle upload after rejecting a contending save', async () => {
  vi.useFakeTimers();
  const f = await fixture();
  try {
    expect(await f.call(IPC.fileSave, { name: 'result.bin', uploadId: f.token })).toMatchObject({ ok: false, error: 'A save is already in progress.' });
    f.finishChoice(); await f.choice;
    await vi.advanceTimersByTimeAsync(120_001);
    await vi.waitFor(() => expect(f.staged.handle.fd).toBe(-1));
    await vi.waitFor(async () => expect(await stat(f.staged.directory).catch(() => null)).toBeNull());
  } finally {
    f.finishChoice(); await f.choice;
    await f.call(IPC.fileSaveAbort, f.token);
    f.sender.emit('destroyed');
    await rm(f.folder, { recursive: true, force: true });
  }
});
