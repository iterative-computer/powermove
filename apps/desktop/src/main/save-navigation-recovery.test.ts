import { EventEmitter } from 'node:events';
import { mkdtemp, writeFile, readFile, rm, stat, type FileHandle } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { afterEach, expect, it, vi } from 'vitest';
import type { IpcMain, IpcMainInvokeEvent } from 'electron';

const native = vi.hoisted(() => ({ showOpen: vi.fn() }));
vi.mock('electron', () => ({ BrowserWindow: { fromWebContents: () => ({ isDestroyed: () => false }) }, dialog: { showOpenDialog: native.showOpen } }));
import { registerSaveIpc, registerProjectOpenPathIpc } from './save';
import { ProjectFiles } from './project-files';
import { IPC } from '../shared/ipc';
import { FileUpload } from './file-upload';

afterEach(() => { vi.restoreAllMocks(); vi.clearAllMocks(); });

it.each([
  { isMainFrame: false, isSameDocument: false },
  { isMainFrame: true, isSameDocument: true }
])('keeps an active save after a navigation that does not replace its document: %j', async details => {
  const f = await fixture();
  let resume!: () => void;
  const pause = new Promise<void>(resolve => { resume = resolve; });
  let choosing = false;
  f.showSave.mockImplementationOnce(async () => { choosing = true; await pause; return { canceled: false, filePath: f.destination }; });
  const bytes = Buffer.from('Current document save');
  const pending = f.call(IPC.fileSave, { name: 'output.png', data: bytes });
  try {
    await vi.waitFor(() => expect(choosing).toBe(true));
    f.sender.emit('did-start-navigation', details);
    resume();
    expect(await pending).toEqual({ ok: true, path: f.destination });
    expect(await readFile(f.destination)).toEqual(bytes);
  } finally {
    resume(); await pending; f.sender.emit('destroyed');
    await rm(f.folder, { recursive: true, force: true });
  }
});

it('closes actual project readers on each document replacement and rearms ownership for the next document', async () => {
  const f = await fixture();
  const projects = new ProjectFiles(path.join(f.folder, 'state', 'registry.json'), path.join(f.folder, 'backups'));
  const source = path.join(f.folder, 'reader.pmv');
  const bytes = Buffer.from(JSON.stringify({ proj: { id: 'nav-document', name: 'Document', w: 100, h: 100, layers: [] } }));
  await writeFile(source, bytes);
  const handlers = new Map<string, (event: IpcMainInvokeEvent, input: unknown) => any>();
  const ipc = { handle: (name: string, handler: any) => handlers.set(name, handler) } as Pick<IpcMain, 'handle'>;
  const ctx = { isTrustedSender: () => true, projects };
  registerSaveIpc(ipc, ctx); registerProjectOpenPathIpc(ipc, ctx);
  const call = (name: string, input: unknown) => handlers.get(name)!({ sender: f.sender } as unknown as IpcMainInvokeEvent, input);
  const tokens: string[] = [];
  try {
    for (let document = 0; document < 2; document++) {
      const opened = await call(IPC.projectOpenPath, source); expect(opened.ok).toBe(true);
      tokens.push(opened.token);
      const handle = (projects as unknown as { readers: Map<string, { handle: FileHandle }> }).readers.get(opened.token)!.handle;
      expect(await call(IPC.projectRead, { token: opened.token, offset: 0, length: bytes.length })).toEqual(bytes);
      f.sender.emit('did-start-navigation', { isMainFrame: true, isSameDocument: false });
      f.sender.emit('did-finish-load');
      await vi.waitFor(() => expect(handle.fd).toBe(-1));
      await expect(call(IPC.projectRead, { token: opened.token, offset: 0, length: 1 })).rejects.toThrow('Unknown project read token');
    }
  } finally {
    f.sender.emit('destroyed');
    for (const token of tokens) await projects.close(token, false);
    await rm(f.folder, { recursive: true, force: true });
  }
});

it.each(['navigation', 'crash'])('does not grant an old Open dialog reader after %s and recovery', async ending => {
  const f = await fixture();
  const projects = new ProjectFiles(path.join(f.folder, 'state', 'registry.json'), path.join(f.folder, 'backups'));
  const source = path.join(f.folder, 'reader.pmv');
  await writeFile(source, JSON.stringify({ proj: { id: 'nav-document', name: 'Document', w: 100, h: 100, layers: [] } }));
  const handlers = new Map<string, (event: IpcMainInvokeEvent, input?: unknown) => any>();
  registerSaveIpc({ handle: (name: string, handler: any) => handlers.set(name, handler) } as Pick<IpcMain, 'handle'>,
    { isTrustedSender: () => true, projects });
  const call = (name: string, input?: unknown) => handlers.get(name)!({ sender: f.sender } as unknown as IpcMainInvokeEvent, input);
  let resume!: () => void;
  const pause = new Promise<void>(resolve => { resume = resolve; });
  let choosing = false;
  native.showOpen.mockImplementationOnce(async () => { choosing = true; await pause; return { canceled: false, filePaths: [source] }; })
    .mockResolvedValue({ canceled: false, filePaths: [source] });
  const tokens: string[] = [];
  const pending = call(IPC.projectOpen).then((result: any) => { if (result.token) tokens.push(result.token); return result; });
  try {
    await vi.waitFor(() => expect(choosing).toBe(true));
    if (ending === 'navigation') f.sender.emit('did-start-navigation', { isMainFrame: true, isSameDocument: false });
    else f.sender.emit('render-process-gone', {}, { reason: 'crashed', exitCode: 1 });
    f.sender.emit('did-finish-load'); resume();
    expect(await pending).toMatchObject({ ok: false, error: 'Project window is unavailable' });
    expect((projects as unknown as { readers: Map<string, unknown> }).readers.size).toBe(0);
    const fresh = await call(IPC.projectOpen); expect(fresh.ok).toBe(true); tokens.push(fresh.token);
    await call(IPC.projectReadClose, fresh.token);
    expect((projects as unknown as { readers: Map<string, unknown> }).readers.size).toBe(0);
  } finally {
    resume(); await pending; f.sender.emit('destroyed');
    for (const token of tokens) await projects.close(token, false);
    await rm(f.folder, { recursive: true, force: true });
  }
});

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

it.each([false, true])('does not publish an old export destination after main-frame reload, directory=%s', async directory => {
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
    f.sender.emit('did-start-navigation', { isMainFrame: true, isSameDocument: false });
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

it('preserves the previous file when a raw save dialog completes after main-frame reload', async () => {
  const f = await fixture();
  let resume!: () => void;
  const pause = new Promise<void>(resolve => { resume = resolve; });
  let choosing = false;
  f.showSave.mockImplementationOnce(async () => { choosing = true; await pause; return { canceled: false, filePath: f.destination }; });
  const pending = f.call(IPC.fileSave, { name: 'output.png', data: Buffer.from('Stale renderer payload') });
  try {
    await vi.waitFor(() => expect(choosing).toBe(true));
    f.sender.emit('did-start-navigation', { isMainFrame: true, isSameDocument: false });
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
    f.sender.emit('did-start-navigation', { isMainFrame: true, isSameDocument: false });
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

it('discards the actual upload spool when its save dialog outlives the old document', async () => {
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
    f.sender.emit('did-start-navigation', { isMainFrame: true, isSameDocument: false });
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
