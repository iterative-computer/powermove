import type { IpcMain, WebContents } from 'electron';

import { IPC, type NativeEditAction } from '../shared/ipc';

const ACTIONS = new Set<NativeEditAction>([
  'undo', 'redo', 'cut', 'copy', 'paste', 'selectAll',
]);

export function registerNativeEditIpc(
  ipcMain: Pick<IpcMain, 'on'>,
  ctx: { isTrustedSenderContents(sender: WebContents): boolean }
): void {
  ipcMain.on(IPC.nativeEdit, (event, value: unknown) => {
    if (!ctx.isTrustedSenderContents(event.sender) || !ACTIONS.has(value as NativeEditAction)) return;
    event.sender[value as NativeEditAction]();
  });
}
