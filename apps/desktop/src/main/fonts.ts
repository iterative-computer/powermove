import type { IpcMain, IpcMainInvokeEvent } from 'electron';
import { fontFamilies } from '@powermove/macos-haptics';
import { IPC } from '../shared/ipc';
import { windowsFontFamilies } from './windows-system';

export function registerFontsIpc(ipc: Pick<IpcMain, 'handle'>, ctx: {
  isTrustedSenderContents(sender: IpcMainInvokeEvent['sender']): boolean;
}) {
  ipc.handle(IPC.fontFamilies, event => {
    if (!ctx.isTrustedSenderContents(event.sender)) return null;
    return process.platform === 'win32' ? windowsFontFamilies() : fontFamilies();
  });
}
