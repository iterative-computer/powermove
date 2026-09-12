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
  const uploads = new Map<object, { id: string; size: number; received: number; chunks: Buffer[]; timer: ReturnType<typeof setTimeout> }>();
  const discardUpload = (owner: object) => { const upload = uploads.get(owner); if (upload) clearTimeout(upload.timer); uploads.delete(owner); };
  ipcMain.handle(IPC.fileSaveUpload, (event, size: unknown) => {
    if (!ctx.isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    if (typeof size !== 'number' || !Number.isSafeInteger(size) || size < 1 || size > LIMITS.fileSaveBytes) throw new Error('Invalid save size');
    if (uploads.has(event.sender) || pending.has(event.sender)) throw new Error('A save is already in progress.');
    const id = randomUUID();
    const timer = setTimeout(() => discardUpload(event.sender), 120_000); timer.unref();
    uploads.set(event.sender, { id, size, received: 0, chunks: [], timer });
    if (!watchedOwners.has(event.sender)) {
      watchedOwners.add(event.sender);
      event.sender.once('destroyed', () => discardUpload(event.sender));
    }
    return id;
  });
  ipcMain.handle(IPC.fileSaveChunk, (event, payload: unknown) => {
    if (!ctx.isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    const upload = uploads.get(event.sender);
    if (!upload || !isRecord(payload) || payload.uploadId !== upload.id || !(payload.data instanceof Uint8Array)
      || payload.data.byteLength < 1 || payload.data.byteLength > 1024 * 1024 || upload.received + payload.data.byteLength > upload.size) throw new Error('Invalid save chunk');
    upload.chunks.push(Buffer.from(payload.data)); upload.received += payload.data.byteLength;
  });
  ipcMain.handle(IPC.fileSaveAbort, (event, id: unknown) => {
    if (!ctx.isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    if (uploads.get(event.sender)?.id === id) discardUpload(event.sender);
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
    if (payload.uploadId !== undefined) {
      const upload = uploads.get(event.sender);
      if (!upload || payload.uploadId !== upload.id || upload.received !== upload.size) throw new Error('The save upload is incomplete.');
      data = Buffer.concat(upload.chunks, upload.size);
      discardUpload(event.sender);
    }
    if (!(data instanceof Uint8Array)) {
      throw new IpcValidationError(IPC.fileSave, 'data must be Uint8Array');
    }
    if (!isBytes(data, LIMITS.fileSaveBytes)) {
      return { ok: false, cancelled: false, error: 'too large; use streaming export' };
    }

    const window = BrowserWindow.fromWebContents(event.sender);
    if (!window || window.isDestroyed()) throw new Error('Save window is unavailable');

    if (pending.has(event.sender)) return { ok: false, cancelled: false, error: 'A save is already in progress.' };
    pending.add(event.sender);
    try {
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
        return { ok: true, path: await ctx.projects.save(projectId, data, selectedPath) };
      }
      await atomicWrite(selectedPath!, data);
      return { ok: true, path: selectedPath! };
    } catch (error: any) {
      return { ok: false, cancelled: false, error: error.message || 'Save failed. Check disk space and folder permissions.' };
    } finally {
      pending.delete(event.sender);
    }
  });
  ipcMain.handle(IPC.projectOpen, async (event): Promise<ProjectOpenResult> => {
    if (!ctx.isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    const window = BrowserWindow.fromWebContents(event.sender);
    if (!window || window.isDestroyed() || !ctx.projects) throw new Error('Project window is unavailable');
    try {
      const result = await dialog.showOpenDialog(window, {
        title: 'Open Project', properties: ['openFile'],
        filters: [{ name: 'Powermove Project', extensions: ['pmv', 'pmv1', 'json'] }]
      });
      if (result.canceled || !result.filePaths[0]) return { ok: false, cancelled: true };
      return { ok: true, ...await ctx.projects.open(result.filePaths[0]) };
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
