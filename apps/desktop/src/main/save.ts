import path from 'node:path';
import { randomUUID } from 'node:crypto';

import {
  BrowserWindow,
  dialog,
  type FileFilter,
  type IpcMain,
  type IpcMainInvokeEvent,
  type SaveDialogOptions,
  type SaveDialogReturnValue
} from 'electron';

import { isBytes, isRecord, isString, IpcValidationError } from '../shared/guards';
import { IPC, LIMITS, PROJECT_ID, type FileSaveResult, type ProjectOpenResult, type CloseDecision } from '../shared/ipc';
import { atomicWrite, type ProjectFiles } from './project-files';
import { FileUpload, FILE_CHUNK_BYTES } from './file-upload';

const MAX_SAVE_NAME_CHARS = 200;

const FILTER_NAMES: Readonly<Record<string, string>> = {
  gif: 'GIF Image',
  jpeg: 'JPEG Image',
  jpg: 'JPEG Image',
  json: 'JSON',
  mp3: 'MP3 Audio',
  mp4: 'MPEG-4 Video',
  pmv: 'Powermove Project',
  png: 'PNG Image',
  svg: 'SVG Image',
  wav: 'WAV Audio',
  webm: 'WebM Video'
};

export interface SaveDialogAdapter {
  showSave(window: BrowserWindow, options: SaveDialogOptions): Promise<SaveDialogReturnValue>;
}

export interface SaveIpcContext {
  isTrustedSender(event: IpcMainInvokeEvent): boolean;
  dialogs?: SaveDialogAdapter;
  projects?: ProjectFiles;
}

/** Convert an untrusted suggestion into one plain filename. */
export function sanitizeSaveName(value: unknown): string | null {
  if (!isString(value)) return null;

  const withoutNul = value.replaceAll('\0', '');
  const components = withoutNul.split(/[\\/]+/).filter(Boolean);
  const name = components.at(-1) ?? '';

  if (
    name === '' ||
    name === '.' ||
    name === '..' ||
    name.length > MAX_SAVE_NAME_CHARS
  ) {
    return null;
  }
  return name;
}

export function saveFiltersForName(name: string): FileFilter[] | undefined {
  const extension = path.extname(name).slice(1).toLowerCase();
  if (!/^[a-z0-9]{1,20}$/.test(extension)) return undefined;

  return [
    {
      name: FILTER_NAMES[extension] ?? `${extension.toUpperCase()} File`,
      extensions: [extension]
    }
  ];
}

export function registerSaveIpc(ipcMain: Pick<IpcMain, 'handle'>, ctx: SaveIpcContext): void {
  const pending = new Set<object>();
  const watchedOwners = new WeakSet<object>();
  const uploads = new Map<object, { id: string; ready: Promise<FileUpload>; timer: ReturnType<typeof setTimeout> }>();
  const readers = new Map<string, { owner: object; timer: ReturnType<typeof setTimeout> }>();
  const discardUpload = async (owner: object, id?: string) => {
    const upload = uploads.get(owner);
    if (!upload || (id && id !== upload.id)) return;
    clearTimeout(upload.timer); uploads.delete(owner);
    const staged = await upload.ready.catch(() => undefined);
    await staged?.dispose();
  };
  const closeReader = async (token: string, verify = false) => {
    const reader = readers.get(token);
    if (!reader) return;
    clearTimeout(reader.timer); readers.delete(token);
    await ctx.projects?.close(token, verify);
  };
  const watchOwner = (owner: IpcMainInvokeEvent['sender']) => {
    if (watchedOwners.has(owner)) return;
    watchedOwners.add(owner);
    owner.once('destroyed', () => {
      void discardUpload(owner).catch(() => undefined);
      for (const [token, reader] of readers) if (reader.owner === owner) void closeReader(token).catch(() => undefined);
    });
  };
  ipcMain.handle(IPC.fileSaveUpload, async (event, size: unknown) => {
    if (!ctx.isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    if (typeof size !== 'number' || !Number.isSafeInteger(size) || size < 1) throw new Error('Invalid save size');
    if (uploads.has(event.sender) || pending.has(event.sender)) throw new Error('A save is already in progress.');
    const id = randomUUID();
    const timer = setTimeout(() => { void discardUpload(event.sender, id).catch(() => undefined); }, 120_000); timer.unref();
    const ready = FileUpload.create(size);
    uploads.set(event.sender, { id, ready, timer }); watchOwner(event.sender);
    try { await ready; return id; }
    catch (error) { await discardUpload(event.sender, id); throw error; }
  });
  ipcMain.handle(IPC.fileSaveChunk, async (event, payload: unknown) => {
    if (!ctx.isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    const upload = uploads.get(event.sender);
    if (!upload || !isRecord(payload) || payload.uploadId !== upload.id || !(payload.data instanceof Uint8Array)
      || payload.data.byteLength < 1 || payload.data.byteLength > FILE_CHUNK_BYTES) throw new Error('Invalid save chunk');
    upload.timer.refresh();
    await (await upload.ready).write(payload.data);
    upload.timer.refresh();
  });
  ipcMain.handle(IPC.fileSaveAbort, async (event, id: unknown) => {
    if (!ctx.isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    if (typeof id === 'string') await discardUpload(event.sender, id);
  });
  ipcMain.handle(IPC.projectRead, async (event, request: unknown) => {
    if (!ctx.isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    if (!isRecord(request) || typeof request.token !== 'string' || readers.get(request.token)?.owner !== event.sender) throw new Error('Unknown project read token');
    const reader = readers.get(request.token)!; reader.timer.refresh();
    const data = await ctx.projects!.read(request.token, request.offset as number, request.length as number);
    reader.timer.refresh(); return data;
  });
  ipcMain.handle(IPC.projectReadClose, async (event, token: unknown) => {
    if (!ctx.isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    if (typeof token !== 'string' || readers.get(token)?.owner !== event.sender) throw new Error('Unknown project read token');
    await closeReader(token, true);
  });
  ipcMain.handle(IPC.fileSave, async (event, payload: unknown): Promise<FileSaveResult> => {
    if (!ctx.isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    if (!isRecord(payload)) throw new IpcValidationError(IPC.fileSave, 'expected an object');

    const name = sanitizeSaveName(payload['name']);
    if (name === null) throw new IpcValidationError(IPC.fileSave, 'invalid name');
    const projectId = payload['projectId'];
    if (projectId !== undefined && (typeof projectId !== 'string' || !PROJECT_ID.test(projectId))) {
      throw new IpcValidationError(IPC.fileSave, 'invalid project id');
    }
    if (payload['saveAs'] !== undefined && typeof payload['saveAs'] !== 'boolean') {
      throw new IpcValidationError(IPC.fileSave, 'invalid saveAs');
    }

    let data = payload['data'];
    let stream: AsyncIterable<Uint8Array> | undefined;
    if (payload.uploadId !== undefined) {
      const upload = uploads.get(event.sender);
      if (!upload || payload.uploadId !== upload.id) throw new Error('The save upload is incomplete.');
      stream = (await upload.ready).claim();
      clearTimeout(upload.timer);
    }
    if (!stream && !(data instanceof Uint8Array)) {
      throw new IpcValidationError(IPC.fileSave, 'data must be Uint8Array');
    }
    if (!stream && !isBytes(data, LIMITS.fileSaveBytes)) {
      return { ok: false, cancelled: false, error: 'too large; use streaming export' };
    }

    if (pending.has(event.sender)) return { ok: false, cancelled: false, error: 'A save is already in progress.' };
    pending.add(event.sender);
    try {
      const window = BrowserWindow.fromWebContents(event.sender);
      if (!window || window.isDestroyed()) throw new Error('Save window is unavailable');
      const known = projectId && ctx.projects ? await ctx.projects.destination(projectId) : undefined;
      let selectedPath: string | undefined;
      if (!known || payload['saveAs']) {
        const options: SaveDialogOptions = { defaultPath: known || name, filters: saveFiltersForName(name) };
        const result = ctx.dialogs?.showSave
          ? await ctx.dialogs.showSave(window, options)
          : await dialog.showSaveDialog(window, options);
        if (result.canceled || !result.filePath) return { ok: false, cancelled: true };
        selectedPath = result.filePath;
      }
      if (projectId && ctx.projects) {
        return { ok: true, path: await ctx.projects.save(projectId, stream || data as Uint8Array, selectedPath) };
      }
      await atomicWrite(selectedPath!, stream || data as Uint8Array);
      return { ok: true, path: selectedPath! };
    } catch (error: any) {
      return { ok: false, cancelled: false, error: error.message || 'Save failed. Check disk space and folder permissions.' };
    } finally {
      pending.delete(event.sender);
      if (typeof payload.uploadId === 'string') await discardUpload(event.sender, payload.uploadId);
    }
  });
  ipcMain.handle(IPC.projectOpen, async (event): Promise<ProjectOpenResult> => {
    if (!ctx.isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    const window = BrowserWindow.fromWebContents(event.sender);
    if (!window || window.isDestroyed() || !ctx.projects) throw new Error('Project window is unavailable');
    try {
      const result = await dialog.showOpenDialog(window, {
        title: 'Open Project', properties: ['openFile'],
        filters: [{ name: 'Powermove Project', extensions: ['pmv', 'json'] }]
      });
      if (result.canceled || !result.filePaths[0]) return { ok: false, cancelled: true };
      const opened = await ctx.projects.open(result.filePaths[0]);
      if (event.sender.isDestroyed()) { await ctx.projects.close(opened.token, false); throw new Error('Project window is unavailable'); }
      const timer = setTimeout(() => { void closeReader(opened.token).catch(() => undefined); }, 120_000); timer.unref();
      readers.set(opened.token, { owner: event.sender, timer }); watchOwner(event.sender);
      return { ok: true, ...opened };
    } catch (error: any) { return { ok: false, cancelled: false, error: error.message || 'Could not open project.' }; }
  });
  ipcMain.handle(IPC.projectConfirmClose, async (event, name: unknown): Promise<CloseDecision> => {
    if (!ctx.isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    if (typeof name !== 'string' || name.length > 1000) throw new IpcValidationError(IPC.projectConfirmClose, 'invalid name');
    const window = BrowserWindow.fromWebContents(event.sender);
    if (!window || window.isDestroyed()) return 'cancel';
    const result = await dialog.showMessageBox(window, {
      type: 'question', message: `Save changes to “${name}” before closing?`,
      detail: 'Your project file has not been updated. Local recovery is kept separately.',
      buttons: ['Save', 'Cancel', 'Don’t Save'], defaultId: 0, cancelId: 1, noLink: true
    });
    return result.response === 0 ? 'save' : result.response === 2 ? 'discard' : 'cancel';
  });
}

export { MAX_SAVE_NAME_CHARS };
