import { randomUUID } from 'node:crypto';
import { rename, unlink, writeFile } from 'node:fs/promises';
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
import { IPC, LIMITS, type FileSaveResult } from '../shared/ipc';

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

async function atomicWrite(filePath: string, data: Uint8Array): Promise<void> {
  const tempPath = path.join(
    path.dirname(filePath),
    `.${path.basename(filePath)}.${process.pid}.${randomUUID()}.tmp`
  );

  try {
    await writeFile(tempPath, data, { flag: 'wx' });
    await rename(tempPath, filePath);
  } catch (error) {
    await unlink(tempPath).catch(() => undefined);
    throw error;
  }
}

export function registerSaveIpc(ipcMain: Pick<IpcMain, 'handle'>, ctx: SaveIpcContext): void {
  ipcMain.handle(IPC.fileSave, async (event, payload: unknown): Promise<FileSaveResult> => {
    if (!ctx.isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    if (!isRecord(payload)) throw new IpcValidationError(IPC.fileSave, 'expected an object');

    const name = sanitizeSaveName(payload['name']);
    if (name === null) throw new IpcValidationError(IPC.fileSave, 'invalid name');

    const data = payload['data'];
    if (!(data instanceof Uint8Array)) {
      throw new IpcValidationError(IPC.fileSave, 'data must be Uint8Array');
    }
    if (!isBytes(data, LIMITS.fileSaveBytes)) {
      return { ok: false, cancelled: false, error: 'too large; use streaming export' };
    }

    const window = BrowserWindow.fromWebContents(event.sender);
    if (!window || window.isDestroyed()) throw new Error('Save window is unavailable');

    const options: SaveDialogOptions = {
      defaultPath: name,
      filters: saveFiltersForName(name)
    };
    const result = ctx.dialogs?.showSave
      ? await ctx.dialogs.showSave(window, options)
      : await dialog.showSaveDialog(window, options);

    if (result.canceled || result.filePath === '') return { ok: false, cancelled: true };

    try {
      await atomicWrite(result.filePath, data);
      return { ok: true, path: result.filePath };
    } catch {
      return { ok: false, cancelled: false, error: 'save failed' };
    }
  });
}

export { MAX_SAVE_NAME_CHARS };
