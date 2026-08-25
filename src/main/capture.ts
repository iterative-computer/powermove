import { BrowserWindow, type IpcMain, type IpcMainInvokeEvent } from 'electron';

import { IPC, type CaptureResult } from '../shared/ipc';

export interface CaptureIpcContext {
  isTrustedSender(event: IpcMainInvokeEvent): boolean;
}

export function registerCaptureIpc(ipcMain: Pick<IpcMain, 'handle'>, ctx: CaptureIpcContext): void {
  ipcMain.handle(IPC.captureWindow, async (event): Promise<CaptureResult> => {
    if (!ctx.isTrustedSender(event)) throw new Error('Unauthorized IPC sender');

    const window = BrowserWindow.fromWebContents(event.sender);
    if (!window || window.isDestroyed() || window.webContents.isDestroyed()) return null;

    try {
      const image = await window.webContents.capturePage();
      return new Uint8Array(image.toPNG());
    } catch {
      return null;
    }
  });
}
