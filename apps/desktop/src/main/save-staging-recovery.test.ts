import { EventEmitter } from 'node:events';
import { mkdtemp, readFile, rm, stat, type FileHandle } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { afterEach, expect, it, vi } from 'vitest';
import type { IpcMain, IpcMainInvokeEvent } from 'electron';

const native = vi.hoisted(() => ({ open: vi.fn() }));
vi.mock('node:fs/promises', async importOriginal => {
  const fs = await importOriginal<typeof import('node:fs/promises')>();
  native.open.mockImplementation(fs.open);
  return { ...fs, open: native.open };
});

vi.mock('electron', () => ({ BrowserWindow: { fromWebContents: () => ({ isDestroyed: () => false }) }, dialog: {} }));
import { ProjectFiles } from './project-files';
import { registerSaveIpc } from './save';
import { FileUpload } from './file-upload';
import { IPC } from '../shared/ipc';
import { decodeProjectContainer } from '../shared/project-container';
import * as durableFile from './durable-file';

afterEach(() => { vi.restoreAllMocks(); vi.clearAllMocks(); });
const document = (name: string) => Buffer.from(JSON.stringify({ proj: { id: 'one', name, w: 100, h: 100, layers: [] } }));

async function fixture() {
  const folder = await mkdtemp(path.join(tmpdir(), 'powermove-save-staging-recovery-'));
  const destination = path.join(folder, 'Project.pmv');
  const before = document('Previous saved work');
  const incoming = document('Recovered save');
  const projects = new ProjectFiles(path.join(folder, 'state', 'registry.json'), path.join(folder, 'backups'));
  await projects.save('one', before, destination);
  native.open.mockClear();
  const originalCreate = FileUpload.create;
  vi.spyOn(FileUpload, 'create').mockImplementation(size => originalCreate(size, folder));
  const sender = Object.assign(new EventEmitter(), { id: 1, isDestroyed: () => false, isCrashed: () => false });
  const handlers = new Map<string, (event: IpcMainInvokeEvent, input: unknown) => any>();
  const showSave = vi.fn(async () => ({ canceled: false, filePath: destination }));
  registerSaveIpc({ handle: (name: string, handler: any) => handlers.set(name, handler) } as Pick<IpcMain, 'handle'>,
    { isTrustedSender: () => true, projects, dialogs: { showSave } });
  const call = (name: string, input: unknown) => handlers.get(name)!({ sender } as unknown as IpcMainInvokeEvent, input);
  const request = { name: 'Project.pmv', projectId: 'one', documentBytes: incoming.length, media: [] };
  const staged = async (kind: string): Promise<{ file: string; handle: FileHandle }> => {
    const index = native.open.mock.calls.findIndex(([file]) => typeof file === 'string' && (kind === 'upload'
      ? path.basename(file) === 'data' : path.basename(file).startsWith('.Project.pmv.') && file.endsWith('.tmp')));
    return { file: native.open.mock.calls[index]![0], handle: await native.open.mock.results[index]!.value };
  };
  return { folder, destination, before, incoming, projects, sender, showSave, call, request, staged };
}

it.each(['incremental', 'upload'])('discards real %s staging on repeated renderer loss and allows a recovered save', async kind => {
  const f = await fixture();
  const tokens: string[] = [];
  const begin = async () => kind === 'upload' ? f.call(IPC.fileSaveUpload, f.incoming.length) : (await f.call(IPC.projectSaveBegin, f.request)).token;
  const chunk = (token: string, bytes: Uint8Array) => f.call(kind === 'upload' ? IPC.fileSaveChunk : IPC.projectSaveChunk,
    kind === 'upload' ? { uploadId: token, data: bytes } : { token, assetId: null, data: bytes });
  try {
    const token = await begin(); tokens.push(token);
    await chunk(token, f.incoming.subarray(0, 2));
    const staged = await f.staged(kind);
    const close = vi.spyOn(staged.handle, 'close');
    f.sender.emit('render-process-gone', {}, { reason: 'crashed', exitCode: 1 });
    f.sender.emit('render-process-gone', {}, { reason: 'crashed', exitCode: 1 });
    f.sender.emit('did-finish-load');
    await vi.waitFor(() => expect(staged.handle.fd).toBe(-1));
    await vi.waitFor(async () => expect(await stat(staged.file).catch(() => null)).toBeNull());
    expect(close).toHaveBeenCalledTimes(1);
    expect(await readFile(f.destination)).toEqual(f.before);
    const fresh = await begin(); tokens.push(fresh);
    expect(fresh).toBeTypeOf('string');
    await chunk(fresh, f.incoming);
    const saved = kind === 'upload'
      ? await f.call(IPC.fileSave, { name: 'Project.pmv', projectId: 'one', uploadId: fresh })
      : await f.call(IPC.projectSaveFinish, fresh);
    expect(saved).toEqual({ ok: true, path: f.destination });
    const bytes = await readFile(f.destination);
    const doc = kind === 'upload' ? JSON.parse(bytes.toString()) : decodeProjectContainer(bytes).document;
    expect(doc.proj.name).toBe('Recovered save');
    f.sender.emit('destroyed');
    expect(close).toHaveBeenCalledTimes(1);
  } finally {
    for (const token of tokens) await f.call(kind === 'upload' ? IPC.fileSaveAbort : IPC.projectSaveAbort, token);
    f.sender.emit('destroyed');
    await rm(f.folder, { recursive: true, force: true });
  }
});

it.each(['dialog', 'incremental-create', 'upload-create'])('does not publish an old staging token after recovery during %s', async stage => {
  const f = await fixture();
  let resume!: () => void;
  const pause = new Promise<void>(resolve => { resume = resolve; });
  let waiting = false;
  if (stage === 'dialog') f.showSave.mockImplementationOnce(async () => { waiting = true; await pause; return { canceled: false, filePath: f.destination }; });
  if (stage === 'incremental-create') {
    const original = f.projects.beginIncremental.bind(f.projects);
    vi.spyOn(f.projects, 'beginIncremental').mockImplementationOnce(async (...args) => {
      const save = await original(...args); waiting = true; await pause; return save;
    });
  }
  if (stage === 'upload-create') {
    const original = FileUpload.create;
    vi.mocked(FileUpload.create).mockImplementationOnce(async size => {
      const upload = await original(size); waiting = true; await pause; return upload;
    });
  }
  let result: { token?: string; error?: Error; ok?: boolean } | undefined;
  const pending = f.call(stage === 'upload-create' ? IPC.fileSaveUpload : IPC.projectSaveBegin,
    stage === 'upload-create' ? f.incoming.length : { ...f.request, saveAs: stage === 'dialog' })
    .then((value: string | { token?: string; ok?: boolean; error?: string }) => {
      result = typeof value === 'string' ? { token: value } : value as any;
    }, (error: Error) => { result = { error }; });
  try {
    await vi.waitFor(() => expect(waiting).toBe(true));
    const staged = stage === 'dialog' ? undefined : await f.staged(stage === 'upload-create' ? 'upload' : 'incremental');
    f.sender.emit('render-process-gone', {}, { reason: 'crashed', exitCode: 1 });
    f.sender.emit('did-finish-load');
    resume(); await pending;
    if (stage === 'upload-create') expect(result).toEqual({ error: expect.objectContaining({ message: 'Save cancelled' }) });
    else expect(result).toEqual({ ok: false, cancelled: false, error: 'Save cancelled' });
    if (staged) {
      await vi.waitFor(() => expect(staged.handle.fd).toBe(-1));
      await vi.waitFor(async () => expect(await stat(staged.file).catch(() => null)).toBeNull());
    }
    expect(await readFile(f.destination)).toEqual(f.before);
  } finally {
    resume(); await pending;
    if (result?.token) await f.call(stage === 'upload-create' ? IPC.fileSaveAbort : IPC.projectSaveAbort, result.token);
    f.sender.emit('destroyed');
    await rm(f.folder, { recursive: true, force: true });
  }
});

it.each(['incremental', 'upload'])('allows an active %s finish to durably commit despite renderer loss', async kind => {
  const f = await fixture();
  let resume!: () => void;
  const pause = new Promise<void>(resolve => { resume = resolve; });
  let committing = false;
  const commits = f.projects as unknown as { commit: (...args: any[]) => Promise<string> };
  const originalCommit = commits.commit.bind(commits);
  vi.spyOn(commits, 'commit').mockImplementationOnce(async (...args) => { committing = true; await pause; return originalCommit(...args); });
  let token: string | undefined;
  let pending: Promise<any> | undefined;
  try {
    token = kind === 'upload' ? await f.call(IPC.fileSaveUpload, f.incoming.length) : (await f.call(IPC.projectSaveBegin, f.request)).token;
    await f.call(kind === 'upload' ? IPC.fileSaveChunk : IPC.projectSaveChunk,
      kind === 'upload' ? { uploadId: token, data: f.incoming } : { token, assetId: null, data: f.incoming });
    const staged = await f.staged(kind);
    pending = kind === 'upload' ? f.call(IPC.fileSave, { name: 'Project.pmv', projectId: 'one', uploadId: token }) : f.call(IPC.projectSaveFinish, token);
    await vi.waitFor(() => expect(committing).toBe(true));
    f.sender.emit('render-process-gone', {}, { reason: 'crashed', exitCode: 1 });
    f.sender.emit('render-process-gone', {}, { reason: 'crashed', exitCode: 1 });
    expect(await readFile(f.destination)).toEqual(f.before);
    resume();
    expect(await pending).toEqual({ ok: true, path: f.destination });
    await vi.waitFor(() => expect(staged.handle.fd).toBe(-1));
    expect(await stat(staged.file).catch(() => null)).toBeNull();
    const bytes = await readFile(f.destination);
    expect((kind === 'upload' ? JSON.parse(bytes.toString()) : decodeProjectContainer(bytes).document).proj.name).toBe('Recovered save');
    const fresh = await f.call(IPC.projectSaveBegin, f.request);
    expect(fresh.ok).toBe(true);
    await f.call(IPC.projectSaveAbort, fresh.token);
  } finally {
    resume(); await pending;
    if (token) await f.call(kind === 'upload' ? IPC.fileSaveAbort : IPC.projectSaveAbort, token);
    f.sender.emit('destroyed');
    await rm(f.folder, { recursive: true, force: true });
  }
});

it('finishes publishing a non-project upload before renderer-loss cleanup removes its source', async () => {
  const f = await fixture();
  let resume!: () => void;
  const pause = new Promise<void>(resolve => { resume = resolve; });
  let copying = false;
  const originalCopy = durableFile.copySnapshot;
  vi.spyOn(durableFile, 'copySnapshot').mockImplementationOnce(async (...args) => { copying = true; await pause; return originalCopy(...args); });
  let token: string | undefined, publishing: Promise<any> | undefined;
  try {
    token = await f.call(IPC.fileSaveUpload, f.incoming.length);
    await f.call(IPC.fileSaveChunk, { uploadId: token, data: f.incoming });
    const staged = await f.staged('upload');
    publishing = f.call(IPC.fileSave, { name: 'Export.bin', uploadId: token });
    await vi.waitFor(() => expect(copying).toBe(true));
    f.sender.emit('render-process-gone', {}, { reason: 'crashed', exitCode: 1 });
    f.sender.emit('did-finish-load');
    expect(staged.handle.fd).not.toBe(-1);
    expect(await readFile(f.destination)).toEqual(f.before);
    resume();
    expect(await publishing).toEqual({ ok: true, path: f.destination });
    await vi.waitFor(() => expect(staged.handle.fd).toBe(-1));
    expect(await stat(staged.file).catch(() => null)).toBeNull();
    expect(await readFile(f.destination)).toEqual(f.incoming);
  } finally {
    resume(); await publishing;
    if (token) await f.call(IPC.fileSaveAbort, token);
    f.sender.emit('destroyed');
    await rm(f.folder, { recursive: true, force: true });
  }
});

it('keeps a recovered request’s lock when an old destination dialog finishes late', async () => {
  const f = await fixture();
  const resumes: Array<() => void> = [];
  f.showSave.mockImplementation(() => new Promise(resolve => { resumes.push(() => resolve({ canceled: false, filePath: f.destination })); }));
  const request = { ...f.request, saveAs: true };
  const old = f.call(IPC.projectSaveBegin, request);
  let fresh: Promise<any> | undefined;
  let freshResult: any;
  try {
    await vi.waitFor(() => expect(resumes.length).toBe(1));
    f.sender.emit('render-process-gone', {}, { reason: 'crashed', exitCode: 1 });
    f.sender.emit('did-finish-load');
    fresh = f.call(IPC.projectSaveBegin, request).then((value: any) => { freshResult = value; return value; }, (error: Error) => ({ error }));
    await vi.waitFor(() => expect(resumes.length).toBe(2));
    resumes[0]!();
    expect(await old).toEqual({ ok: false, cancelled: false, error: 'Save cancelled' });
    await expect(f.call(IPC.projectSaveBegin, request)).rejects.toThrow('A save is already in progress');
    expect(f.showSave).toHaveBeenCalledTimes(2);
    expect(await readFile(f.destination)).toEqual(f.before);
    resumes[1]!();
    const result = await fresh;
    expect(result.ok).toBe(true);
    await f.call(IPC.projectSaveChunk, { token: result.token, assetId: null, data: f.incoming });
    expect(await f.call(IPC.projectSaveFinish, result.token)).toEqual({ ok: true, path: f.destination });
  } finally {
    // New dialogs can only appear after a preceding continuation resumes.
    for (const resume of resumes) resume();
    const oldResult = await old;
    if (oldResult.token) await f.call(IPC.projectSaveAbort, oldResult.token);
    if (fresh) await fresh;
    if (freshResult?.token) await f.call(IPC.projectSaveAbort, freshResult.token);
    f.sender.emit('destroyed');
    await rm(f.folder, { recursive: true, force: true });
  }
});

it('keeps a recovered dialog’s lock when the old raw upload finish completes', async () => {
  const f = await fixture();
  let finishResume!: () => void;
  let dialogResume!: () => void;
  const finishPause = new Promise<void>(resolve => { finishResume = resolve; });
  const dialogPause = new Promise<void>(resolve => { dialogResume = resolve; });
  let committing = false, choosing = false;
  const commits = f.projects as unknown as { commit: (...args: any[]) => Promise<string> };
  const originalCommit = commits.commit.bind(commits);
  vi.spyOn(commits, 'commit').mockImplementationOnce(async (...args) => { committing = true; await finishPause; return originalCommit(...args); });
  f.showSave.mockImplementationOnce(async () => { choosing = true; await dialogPause; return { canceled: false, filePath: f.destination }; });
  let token: string | undefined;
  let old: Promise<any> | undefined, fresh: Promise<any> | undefined;
  let freshResult: any, extraResult: any;
  try {
    token = await f.call(IPC.fileSaveUpload, f.incoming.length);
    await f.call(IPC.fileSaveChunk, { uploadId: token, data: f.incoming });
    const staged = await f.staged('upload');
    old = f.call(IPC.fileSave, { name: 'Project.pmv', projectId: 'one', uploadId: token });
    await vi.waitFor(() => expect(committing).toBe(true));
    f.sender.emit('render-process-gone', {}, { reason: 'crashed', exitCode: 1 });
    f.sender.emit('did-finish-load');
    await vi.waitFor(() => expect(staged.handle.fd).toBe(-1));
    await vi.waitFor(async () => expect(await stat(staged.file).catch(() => null)).toBeNull());
    const request = { ...f.request, saveAs: true };
    fresh = f.call(IPC.projectSaveBegin, request).then((value: any) => { freshResult = value; return value; }, (error: Error) => ({ error }));
    await vi.waitFor(() => expect(choosing).toBe(true));
    finishResume();
    expect(await old).toEqual({ ok: true, path: f.destination });
    extraResult = await f.call(IPC.projectSaveBegin, request).then((value: any) => value, (error: Error) => ({ error }));
    expect(extraResult).toEqual({ error: expect.objectContaining({ message: 'A save is already in progress.' }) });
    dialogResume();
    expect((await fresh).ok).toBe(true);
    await f.call(IPC.projectSaveAbort, freshResult.token);
    expect(JSON.parse((await readFile(f.destination)).toString()).proj.name).toBe('Recovered save');
  } finally {
    finishResume(); dialogResume();
    await old; await fresh;
    if (token) await f.call(IPC.fileSaveAbort, token);
    for (const result of [freshResult, extraResult]) if (result?.token) await f.call(IPC.projectSaveAbort, result.token);
    f.sender.emit('destroyed');
    await rm(f.folder, { recursive: true, force: true });
  }
});

it.each(['incremental', 'upload'])('waits for an in-flight %s write without rearming its cancelled timer', async kind => {
  const f = await fixture();
  const timers = vi.spyOn(globalThis, 'setTimeout');
  let resume!: () => void;
  const pause = new Promise<void>(resolve => { resume = resolve; });
  let token: string | undefined, pending: Promise<any> | undefined;
  let writing = false;
  try {
    token = kind === 'upload' ? await f.call(IPC.fileSaveUpload, f.incoming.length) : (await f.call(IPC.projectSaveBegin, f.request)).token;
    const staged = await f.staged(kind);
    const timerIndex = timers.mock.calls.findIndex(call => call[1] === 120_000);
    const timer = timers.mock.results[timerIndex]!.value as ReturnType<typeof setTimeout>;
    const refresh = vi.spyOn(timer, 'refresh');
    const originalWrite = staged.handle.write.bind(staged.handle);
    vi.spyOn(staged.handle, 'write').mockImplementationOnce(async (...args: any[]) => { writing = true; await pause; return (originalWrite as any)(...args); });
    pending = f.call(kind === 'upload' ? IPC.fileSaveChunk : IPC.projectSaveChunk,
      kind === 'upload' ? { uploadId: token, data: f.incoming } : { token, assetId: null, data: f.incoming });
    await vi.waitFor(() => expect(writing).toBe(true));
    const refreshes = refresh.mock.calls.length;
    f.sender.emit('render-process-gone', {}, { reason: 'crashed', exitCode: 1 });
    f.sender.emit('render-process-gone', {}, { reason: 'crashed', exitCode: 1 });
    expect(staged.handle.fd).toBeGreaterThanOrEqual(0);
    resume(); await pending;
    await vi.waitFor(() => expect(staged.handle.fd).toBe(-1));
    await vi.waitFor(async () => expect(await stat(staged.file).catch(() => null)).toBeNull());
    expect(refresh).toHaveBeenCalledTimes(refreshes);
    expect(await readFile(f.destination)).toEqual(f.before);
  } finally {
    resume(); await pending;
    if (token) await f.call(kind === 'upload' ? IPC.fileSaveAbort : IPC.projectSaveAbort, token);
    f.sender.emit('destroyed');
    await rm(f.folder, { recursive: true, force: true });
  }
});
