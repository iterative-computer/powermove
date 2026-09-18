import { BrowserWindow, dialog, type IpcMain, type IpcMainInvokeEvent } from 'electron';

import { IPC, type ConfirmRequest } from '../shared/ipc';

export interface ConfirmIpcContext {
  isTrustedSender(event: IpcMainInvokeEvent): boolean;
  /** Test seam; defaults to dialog.showMessageBox. */
  showMessageBox?: typeof dialog.showMessageBox;
}

const MAX_TEXT = 2000;
const MAX_LABEL = 60;

function text(value: unknown, max: number): string | null {
  if (typeof value !== 'string') return null;
  const trimmed = value.trim();
  return trimmed.length > 0 && trimmed.length <= max ? trimmed : null;
}

/**
 * Plain yes-or-no prompts are native sheets on macOS, never DOM overlays.
 * Button order follows the HIG: the action button sits on the right
 * (index 0), Cancel to its left. Destructive prompts default to Cancel so
 * Return does not delete anything.
 */
export function confirmOptions(request: ConfirmRequest): Electron.MessageBoxOptions {
  const confirmLabel = text(request.confirmLabel, MAX_LABEL) ?? 'OK';
  const detail = text(request.detail, MAX_TEXT);
  return {
    type: request.destructive ? 'warning' : 'question',
    message: request.message,
    ...(detail ? { detail } : {}),
    buttons: [confirmLabel, 'Cancel'],
    defaultId: request.destructive ? 1 : 0,
    cancelId: 1,
    noLink: true
  };
}

export function registerConfirmIpc(ipcMain: Pick<IpcMain, 'handle'>, ctx: ConfirmIpcContext): void {
  const show = ctx.showMessageBox ?? ((window, options) => dialog.showMessageBox(window, options));
  ipcMain.handle(IPC.dialogConfirm, async (event: IpcMainInvokeEvent, request: unknown): Promise<boolean> => {
    if (!ctx.isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    const message = text((request as ConfirmRequest | null)?.message, MAX_TEXT);
    if (!message) throw new Error('dialog:confirm needs a message');
    const window = BrowserWindow.fromWebContents(event.sender);
    if (!window || window.isDestroyed()) return false;
    const input = request as ConfirmRequest;
    const result = await show(window, confirmOptions({ ...input, message }));
    return result.response === 0;
  });
}
