import { nativeTheme, type IpcMain, type IpcMainEvent, type NativeTheme } from 'electron';

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
  });
}

export { DARK_BACKGROUND, LIGHT_BACKGROUND, THEME_SOURCES };
