import { EventEmitter } from 'node:events';
import { mkdtemp, writeFile, readFile, rm, stat, type FileHandle } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { afterEach, expect, it, vi } from 'vitest';
import type { IpcMain, IpcMainInvokeEvent } from 'electron';

const native = vi.hoisted(() => ({ showOpen: vi.fn() }));
vi.mock('electron', () => ({ BrowserWindow: { fromWebContents: () => ({ isDestroyed: () => false }) }, dialog: { showOpenDialog: native.showOpen } }));
import { registerSaveIpc } from './save';
import { IPC } from '../shared/ipc';
import { FileUpload } from './file-upload';

afterEach(() => { vi.restoreAllMocks(); vi.clearAllMocks(); });

async function fixture() {
  const folder = await mkdtemp(path.join(tmpdir(), 'powermove-save-dialog-recovery-'));
  const destination = path.join(folder, 'output.png');
  const before = Buffer.from('Previously saved work');
  await writeFile(destination, before);
  const sender = Object.assign(new EventEmitter(), { id: 1, isDestroyed: () => false, isCrashed: () => false });
  const handlers = new Map<string, (event: IpcMainInvokeEvent, input: unknown) => any>();
  const showSave = vi.fn(async () => ({ canceled: false, filePath: destination }));
  registerSaveIpc({ handle: (name: string, handler: any) => handlers.set(name, handler) } as Pick<IpcMain, 'handle'>,
    { isTrustedSender: () => true, dialogs: { showSave } });
  const call = (name: string, input: unknown) => handlers.get(name)!({ sender } as unknown as IpcMainInvokeEvent, input);
  return { folder, destination, before, sender, showSave, call };
}

it.each([false, true])('does not publish an old export destination after recovery, directory=%s', async directory => {
  const f = await fixture();
  let resume!: () => void;
  const pause = new Promise<void>(resolve => { resume = resolve; });
  let choosing = false;
  if (directory) native.showOpen.mockImplementationOnce(async () => { choosing = true; await pause; return { canceled: false, filePaths: [f.folder] }; }).mockResolvedValue({ canceled: false, filePaths: [f.folder] });
  else f.showSave.mockImplementationOnce(async () => { choosing = true; await pause; return { canceled: false, filePath: f.destination }; });
  let result: { token?: string | null; error?: Error } | undefined;
  const pending = f.call(IPC.exportChoose, { name: 'output.png', directory })
    .then((token: string | null) => { result = { token }; }, (error: Error) => { result = { error }; });
  let fresh: string | undefined;
  try {
    await vi.waitFor(() => expect(choosing).toBe(true));
    f.sender.emit('render-process-gone', {}, { reason: 'crashed', exitCode: 1 });
    f.sender.emit('did-finish-load');
    resume(); await pending;
    expect(result?.token ?? null).toBeNull();
    expect(await readFile(f.destination)).toEqual(f.before);
    fresh = await f.call(IPC.exportChoose, { name: 'output.png', directory });
    expect(fresh).toBeTypeOf('string');
    const bytes = new Uint8Array([7, 8, 9]);
    expect(await f.call(IPC.fileSave, { name: 'output.png', data: bytes, destinationToken: fresh })).toEqual({ ok: true, path: f.destination });
    expect(await readFile(f.destination)).toEqual(Buffer.from(bytes));
  } finally {
    resume(); await pending;
    for (const token of [result?.token, fresh]) if (token) await f.call(IPC.exportRelease, token);
    f.sender.emit('destroyed');
    await rm(f.folder, { recursive: true, force: true });
  }
});

it('preserves the previous file when a raw save dialog completes after renderer recovery', async () => {
  const f = await fixture();
  let resume!: () => void;
  const pause = new Promise<void>(resolve => { resume = resolve; });
  let choosing = false;
  f.showSave.mockImplementationOnce(async () => { choosing = true; await pause; return { canceled: false, filePath: f.destination }; });
  const pending = f.call(IPC.fileSave, { name: 'output.png', data: Buffer.from('Stale renderer payload') });
  try {
    await vi.waitFor(() => expect(choosing).toBe(true));
    f.sender.emit('render-process-gone', {}, { reason: 'crashed', exitCode: 1 });
    f.sender.emit('did-finish-load');
    resume();
    const result = await pending;
    expect(await readFile(f.destination)).toEqual(f.before);
    expect(result).toMatchObject({ ok: false });
    const bytes = Buffer.from('Fresh renderer payload');
    expect(await f.call(IPC.fileSave, { name: 'output.png', data: bytes })).toEqual({ ok: true, path: f.destination });
    expect(await readFile(f.destination)).toEqual(bytes);
    expect(f.sender.listenerCount('render-process-gone')).toBeLessThanOrEqual(1);
  } finally {
    resume(); await pending;
    f.sender.emit('destroyed');
    await rm(f.folder, { recursive: true, force: true });
  }
});

it.each([false, true])('preserves a recovered export grant when the old dialog finishes late, directory=%s', async directory => {
  const f = await fixture();
  let resume!: () => void;
  const pause = new Promise<void>(resolve => { resume = resolve; });
  let choosing = false;
  if (directory) native.showOpen.mockImplementationOnce(async () => { choosing = true; await pause; return { canceled: false, filePaths: [f.folder] }; }).mockResolvedValue({ canceled: false, filePaths: [f.folder] });
  else f.showSave.mockImplementationOnce(async () => { choosing = true; await pause; return { canceled: false, filePath: f.destination }; });
  const old = f.call(IPC.exportChoose, { name: 'output.png', directory });
  let fresh: string | undefined, oldToken: string | null | undefined;
  try {
    await vi.waitFor(() => expect(choosing).toBe(true));
    f.sender.emit('render-process-gone', {}, { reason: 'crashed', exitCode: 1 });
    f.sender.emit('did-finish-load');
    fresh = await f.call(IPC.exportChoose, { name: 'output.png', directory });
    expect(fresh).toBeTypeOf('string');
    resume(); oldToken = await old;
    expect(oldToken).toBeNull();
    const bytes = new Uint8Array([42]);
    expect(await f.call(IPC.fileSave, { name: 'output.png', data: bytes, destinationToken: fresh })).toEqual({ ok: true, path: f.destination });
    expect(await readFile(f.destination)).toEqual(Buffer.from(bytes));
  } finally {
    resume(); oldToken = await old;
    for (const token of [fresh, oldToken]) if (token) await f.call(IPC.exportRelease, token);
    f.sender.emit('destroyed');
    await rm(f.folder, { recursive: true, force: true });
  }
});

it('discards the actual upload spool when its save dialog outlives the renderer', async () => {
  const f = await fixture();
  const originalCreate = FileUpload.create;
  let spool!: FileUpload;
  vi.spyOn(FileUpload, 'create').mockImplementation(async size => { spool = await originalCreate(size, f.folder); return spool; });
  let resume!: () => void;
  const pause = new Promise<void>(resolve => { resume = resolve; });
  let choosing = false;
  f.showSave.mockImplementationOnce(async () => { choosing = true; await pause; return { canceled: false, filePath: f.destination }; });
  const bytes = Buffer.from('Stale uploaded bytes');
  const token = await f.call(IPC.fileSaveUpload, bytes.length);
  await f.call(IPC.fileSaveChunk, { uploadId: token, data: bytes });
  const staged = spool as unknown as { handle: FileHandle; directory: string };
  const close = vi.spyOn(staged.handle, 'close');
  const pending = f.call(IPC.fileSave, { name: 'output.png', uploadId: token });
  try {
    await vi.waitFor(() => expect(choosing).toBe(true));
    f.sender.emit('render-process-gone', {}, { reason: 'crashed', exitCode: 1 });
    f.sender.emit('did-finish-load');
    resume();
    const result = await pending;
    expect(await readFile(f.destination)).toEqual(f.before);
    expect(result).toMatchObject({ ok: false });
    await vi.waitFor(() => expect(staged.handle.fd).toBe(-1));
    await vi.waitFor(async () => expect(await stat(staged.directory).catch(() => null)).toBeNull());
    expect(close).toHaveBeenCalledTimes(1);
  } finally {
    resume(); await pending;
    await f.call(IPC.fileSaveAbort, token);
    f.sender.emit('destroyed');
    await rm(f.folder, { recursive: true, force: true });
  }
});
