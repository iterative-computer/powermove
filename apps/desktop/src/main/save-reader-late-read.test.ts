import { EventEmitter } from 'node:events';
import { mkdtemp, writeFile, rm, type FileHandle } from 'node:fs/promises';
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
vi.mock('electron', () => ({ BrowserWindow: {}, dialog: {} }));
import { ProjectFiles } from './project-files';
import { registerProjectOpenPathIpc, registerSaveIpc } from './save';
import { IPC } from '../shared/ipc';

afterEach(() => { vi.useRealTimers(); vi.restoreAllMocks(); vi.clearAllMocks(); });

it('does not rearm a closed reader timer when its in-flight native read returns', async () => {
  const folder = await mkdtemp(path.join(tmpdir(), 'powermove-project-reader-late-read-'));
  const source = path.join(folder, 'source.pmv');
  const bytes = Buffer.from(JSON.stringify({ proj: { id: 'embedded', name: 'Source', w: 100, h: 100, layers: [] } }));
  await writeFile(source, bytes);
  const projects = new ProjectFiles(path.join(folder, 'state', 'registry.json'), path.join(folder, 'backups'));
  const handlers = new Map<string, (event: IpcMainInvokeEvent, input: unknown) => any>();
  const ipc = { handle: (name: string, handler: any) => handlers.set(name, handler) } as Pick<IpcMain, 'handle'>;
  registerSaveIpc(ipc, { isTrustedSender: () => true, projects });
  registerProjectOpenPathIpc(ipc, { isTrustedSender: () => true, projects });
  const sender = Object.assign(new EventEmitter(), { id: 1, isDestroyed: () => false, isCrashed: () => false });
  const call = (name: string, input: unknown) => handlers.get(name)!({ sender } as unknown as IpcMainInvokeEvent, input);
  vi.useFakeTimers();
  const opened = await call(IPC.projectOpenPath, source);
  expect(opened.ok).toBe(true);
  const index = native.open.mock.calls.map(call => call[0]).lastIndexOf(source);
  const handle: FileHandle = await native.open.mock.results[index]!.value;
  let resume!: () => void;
  const pause = new Promise<void>(resolve => { resume = resolve; });
  let readDone!: () => void;
  const ready = new Promise<void>(resolve => { readDone = resolve; });
  const originalRead = projects.read.bind(projects);
  vi.spyOn(projects, 'read').mockImplementationOnce(async (token, offset, length) => {
    const data = await originalRead(token, offset, length);
    readDone(); await pause;
    return data;
  });
  const pending = call(IPC.projectRead, { token: opened.token, offset: 0, length: bytes.length });
  try {
    await ready;
    expect(vi.getTimerCount()).toBe(1);
    await call(IPC.projectReadClose, opened.token);
    expect(handle.fd).toBe(-1);
    expect(vi.getTimerCount()).toBe(0);
    resume();
    expect(await pending).toEqual(bytes);
    // A removed reader must not regain an idle timer that retains its owner
    // and project registry for another two minutes after closing.
    expect(vi.getTimerCount()).toBe(0);
    await expect(call(IPC.projectRead, { token: opened.token, offset: 0, length: 1 })).rejects.toThrow('Unknown project read token');
  } finally {
    resume(); await pending;
    sender.emit('destroyed');
    await projects.close(opened.token, false);
    await rm(folder, { recursive: true, force: true });
  }
});
