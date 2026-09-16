import { shell, type IpcMain, type IpcMainInvokeEvent } from 'electron';
import { createHash } from 'node:crypto';
import { mkdir, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';

import { isBytes, isRecord, isString, IpcValidationError } from '../shared/guards';
import { IPC } from '../shared/ipc';

export interface ShellIpcContext {
  isTrustedSender(event: IpcMainInvokeEvent): boolean;
  attachmentCacheDirectory?: string;
}

function attachmentName(value: unknown): string | null {
  if (!isString(value, 255) || value.length === 0 || value.includes('\0') || value === '.' || value === '..') return null;
  if (path.basename(value) !== value || value.includes('\\')) return null;
  return value;
}

export async function materializeAttachment(cacheDirectory: string, raw: unknown): Promise<string> {
  if (!isRecord(raw)) throw new IpcValidationError(IPC.attachmentReveal, 'expected attachment bytes');
  const name = attachmentName(raw.name);
  if (name === null || !isBytes(raw.data)) {
    throw new IpcValidationError(IPC.attachmentReveal, 'expected a safe name and attachment bytes');
  }
  await mkdir(cacheDirectory, { recursive: true });
  const digest = createHash('sha256').update(raw.data).digest('hex').slice(0, 16);
  const targetDirectory = path.join(cacheDirectory, digest);
  await mkdir(targetDirectory, { recursive: true });
  const filePath = path.join(targetDirectory, name);
  try { await writeFile(filePath, raw.data, { flag: 'wx' }); }
  catch (error) {
    if ((error as NodeJS.ErrnoException).code !== 'EEXIST') throw error;
  }
  return filePath;
}

export function parseExternalUrl(value: unknown): URL | null {
  if (!isString(value)) return null;
  try {
    const url = new URL(value);
    return url.protocol === 'http:' || url.protocol === 'https:' ? url : null;
  } catch {
    return null;
  }
}

export function registerShellIpc(ipcMain: Pick<IpcMain, 'handle'>, ctx: ShellIpcContext): void {
  ipcMain.handle(IPC.openExternal, async (event, value: unknown): Promise<void> => {
    if (!ctx.isTrustedSender(event)) throw new Error('Unauthorized IPC sender');

    const url = parseExternalUrl(value);
    if (url === null) throw new IpcValidationError(IPC.openExternal, 'expected an http(s) URL');
    await shell.openExternal(url.toString());
  });

  ipcMain.handle(IPC.mediaRevealSource, (event, value: unknown): void => {
    if (!ctx.isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    if (!isString(value) || value.length > 16_384 || !path.isAbsolute(value)) {
      throw new IpcValidationError(IPC.mediaRevealSource, 'expected an absolute file path');
    }
    shell.showItemInFolder(path.normalize(value));
  });

  ipcMain.handle(IPC.attachmentReveal, async (event, value: unknown): Promise<void> => {
    if (!ctx.isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    const cacheDirectory = ctx.attachmentCacheDirectory ?? path.join(tmpdir(), 'Powermove Attachment Cache');
    shell.showItemInFolder(await materializeAttachment(cacheDirectory, value));
  });
}
