import { BrowserWindow, nativeTheme, type IpcMain, type IpcMainEvent, type NativeTheme } from 'electron';

import { isOneOf } from '../shared/guards';
import { IPC, type ThemeSource } from '../shared/ipc';

const THEME_SOURCES = ['light', 'dark', 'system'] as const;
const DARK_BACKGROUND = '#0c0a09';
const LIGHT_BACKGROUND = '#ededef';

export interface ThemeIpcContext {
  isTrustedSenderContents(sender: IpcMainEvent['sender']): boolean;
}

export function currentBackgroundColor(
  theme: ThemeSource | Pick<NativeTheme, 'shouldUseDarkColors'>
): string {
  const isDark =
    typeof theme === 'string'
      ? theme === 'dark' || (theme === 'system' && nativeTheme.shouldUseDarkColors)
      : theme.shouldUseDarkColors;
  return isDark ? DARK_BACKGROUND : LIGHT_BACKGROUND;
}

export function registerThemeIpc(ipcMain: Pick<IpcMain, 'on'>, ctx: ThemeIpcContext): void {
  ipcMain.on(IPC.themeSet, (event, value: unknown) => {
    if (!ctx.isTrustedSenderContents(event.sender)) return;
    if (!isOneOf(value, THEME_SOURCES)) return;
    nativeTheme.themeSource = value;
    if (process.platform === 'win32') for (const window of BrowserWindow.getAllWindows()) {
      if (!window.isDestroyed()) window.setTitleBarOverlay({ color: currentBackgroundColor(value), symbolColor: currentBackgroundColor(value) === DARK_BACKGROUND ? '#f5f5f5' : '#171717', height: 44 });
    }
  });
}

export { DARK_BACKGROUND, LIGHT_BACKGROUND, THEME_SOURCES };
