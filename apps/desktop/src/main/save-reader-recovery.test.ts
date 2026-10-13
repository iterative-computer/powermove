import { EventEmitter } from 'node:events';
import { mkdtemp, writeFile, readFile, rm, type FileHandle } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { afterEach, expect, it, vi } from 'vitest';
import type { IpcMain, IpcMainInvokeEvent, WebContents } from 'electron';

const native = vi.hoisted(() => ({ open: vi.fn() }));
vi.mock('node:fs/promises', async importOriginal => {
  const fs = await importOriginal<typeof import('node:fs/promises')>();
  native.open.mockImplementation(fs.open);
  return { ...fs, open: native.open };
});
vi.mock('electron', () => ({ BrowserWindow: {}, dialog: {} }));
import { ProjectFiles } from './project-files';
import { openProjectForWindow, registerProjectOpenPathIpc, registerSaveIpc } from './save';
import { IPC } from '../shared/ipc';

afterEach(() => { vi.restoreAllMocks(); vi.clearAllMocks(); });

async function fixture() {
  const folder = await mkdtemp(path.join(tmpdir(), 'powermove-project-reader-recovery-'));
  const source = path.join(folder, 'source.pmv');
  const bytes = Buffer.from(JSON.stringify({ proj: { id: 'embedded', name: 'Source', w: 100, h: 100, layers: [] } }));
  await writeFile(source, bytes);
  const projects = new ProjectFiles(path.join(folder, 'state', 'registry.json'), path.join(folder, 'backups'));
  const handlers = new Map<string, (event: IpcMainInvokeEvent, input: unknown) => any>();
  const ipc = { handle: (name: string, handler: any) => handlers.set(name, handler) } as Pick<IpcMain, 'handle'>;
  registerSaveIpc(ipc, { isTrustedSender: () => true, projects });
  registerProjectOpenPathIpc(ipc, { isTrustedSender: () => true, projects });
  let destroyed = false;
  const sender = Object.assign(new EventEmitter(), { id: 1, isDestroyed: () => destroyed, isCrashed: () => false });
  const call = (name: string, input: unknown) => handlers.get(name)!({ sender } as unknown as IpcMainInvokeEvent, input);
  const handle = async (): Promise<FileHandle> => {
    const index = native.open.mock.calls.map(call => call[0]).lastIndexOf(source);
    return native.open.mock.results[index]!.value;
  };
  return { folder, source, bytes, projects, sender, call, handle, destroy: () => { destroyed = true; sender.emit('destroyed'); } };
}

it('closes a real project reader once after a renderer crash and permits fresh recovery reads', async () => {
  const f = await fixture();
  const tokens: string[] = [];
  try {
    const opened = await f.call(IPC.projectOpenPath, f.source);
    expect(opened.ok).toBe(true); tokens.push(opened.token);
    const handle = await f.handle();
    const close = vi.spyOn(handle, 'close');
    expect(new TextDecoder().decode(await f.call(IPC.projectRead, { token: opened.token, offset: 0, length: f.bytes.length }))).toBe(f.bytes.toString());
    f.sender.emit('render-process-gone', {}, { reason: 'crashed', exitCode: 1 });
    expect(f.sender.isDestroyed()).toBe(false);
    await vi.waitFor(() => expect(handle.fd).toBe(-1));
    expect(close).toHaveBeenCalledTimes(1);
    await expect(f.call(IPC.projectRead, { token: opened.token, offset: 0, length: 1 })).rejects.toThrow('Unknown project read token');
    f.sender.emit('did-finish-load');
    const fresh = await f.call(IPC.projectOpenPath, f.source);
    expect(fresh.ok).toBe(true); tokens.push(fresh.token);
    const freshHandle = await f.handle();
    const freshClose = vi.spyOn(freshHandle, 'close');
    expect(new TextDecoder().decode(await f.call(IPC.projectRead, { token: fresh.token, offset: 0, length: f.bytes.length }))).toBe(f.bytes.toString());
    f.sender.emit('render-process-gone', {}, { reason: 'crashed', exitCode: 1 });
    await vi.waitFor(() => expect(freshHandle.fd).toBe(-1));
    expect(freshClose).toHaveBeenCalledTimes(1);
    f.destroy();
    expect(close).toHaveBeenCalledTimes(1);
    expect(freshClose).toHaveBeenCalledTimes(1);
    expect(await readFile(f.source)).toEqual(f.bytes);
  } finally {
    f.destroy();
    for (const token of tokens) await f.projects.close(token, false);
    await rm(f.folder, { recursive: true, force: true });
  }
});

it('preserves a late document-validation failure after the renderer crashes and recovers', async () => {
  const f = await fixture();
  await writeFile(f.source, JSON.stringify({ proj: { name: 'Invalid project' } }));
  let resume!: () => void;
  const pause = new Promise<void>(resolve => { resume = resolve; });
  let failed = false;
  const originalOpen = f.projects.open.bind(f.projects);
  vi.spyOn(f.projects, 'open').mockImplementationOnce(async (file, options) => {
    try { return await originalOpen(file, options!); }
    catch (error) { failed = true; await pause; throw error; }
  });
  const pending = openProjectForWindow({ projects: f.projects }, f.sender as unknown as WebContents, f.source);
  try {
    await vi.waitFor(() => expect(failed).toBe(true));
    f.sender.emit('render-process-gone', {}, { reason: 'crashed', exitCode: 1 });
    f.sender.emit('did-finish-load');
    resume();
    expect(await pending).toEqual({ ok: false, cancelled: false, error: 'This is not a Powermove project.' });
    expect((await f.handle()).fd).toBe(-1);
    expect(f.sender.listenerCount('render-process-gone')).toBe(0);
  } finally {
    resume(); await pending;
    f.destroy();
    await rm(f.folder, { recursive: true, force: true });
  }
});

it.each([
  { reason: 'crashed', cleanupFails: false },
  { reason: 'crashed', cleanupFails: true },
  { reason: 'destroyed', cleanupFails: true },
])('rejects a late project reader after $reason, preserving the window error when cleanup fails=$cleanupFails', async ({ reason, cleanupFails }) => {
  const f = await fixture();
  let resume!: () => void;
  const pause = new Promise<void>(resolve => { resume = resolve; });
  let opened: Awaited<ReturnType<ProjectFiles['open']>> | undefined;
  const originalOpen = f.projects.open.bind(f.projects);
  vi.spyOn(f.projects, 'open').mockImplementationOnce(async (file, options) => {
    opened = await originalOpen(file, options!);
    await pause;
    return opened;
  });
  const originalClose = f.projects.close.bind(f.projects);
  if (cleanupFails) vi.spyOn(f.projects, 'close').mockImplementation(async (token, verify) => {
    await originalClose(token, verify);
    throw new Error('Project reader cleanup failed');
  });
  const pending = openProjectForWindow({ projects: f.projects }, f.sender as unknown as WebContents, f.source);
  try {
    await vi.waitFor(() => expect(opened?.token).toBeTypeOf('string'));
    const handle = await f.handle();
    const close = vi.spyOn(handle, 'close');
    if (reason === 'destroyed') f.destroy();
    else {
      f.sender.emit('render-process-gone', {}, { reason: 'crashed', exitCode: 1 });
      f.sender.emit('did-finish-load');
    }
    resume();
    expect(await pending).toEqual({ ok: false, cancelled: false, error: 'Project window is unavailable' });
    expect(handle.fd).toBe(-1);
    expect(close).toHaveBeenCalledTimes(1);
    expect(await readFile(f.source)).toEqual(f.bytes);
  } finally {
    resume(); await pending;
    if (opened) await originalClose(opened.token, false);
    f.destroy();
    await rm(f.folder, { recursive: true, force: true });
  }
});
