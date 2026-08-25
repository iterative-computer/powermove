import { shell, type IpcMain, type IpcMainInvokeEvent } from 'electron';

import { isString, IpcValidationError } from '../shared/guards';
import { IPC } from '../shared/ipc';

export interface ShellIpcContext {
  isTrustedSender(event: IpcMainInvokeEvent): boolean;
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
}
