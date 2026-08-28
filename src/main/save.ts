import path from 'node:path';

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

    const data = payload['data'];
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
