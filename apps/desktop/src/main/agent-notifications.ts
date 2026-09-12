import { app, BrowserWindow, Notification, type IpcMain, type WebContents } from 'electron';
import path from 'node:path';
import { execFile } from 'node:child_process';
import { AGENT_SOUNDS } from '../shared/agent-notifications';

export function registerAgentNotifications(ipc: Pick<IpcMain, 'handle'>, ctx: {
  isTrustedSenderContents(sender: WebContents): boolean;
}): void {
  ipc.handle('agent:notification', (event, options) => {
    if (!ctx.isTrustedSenderContents(event.sender)) return;
    if (!options || !AGENT_SOUNDS.includes(options.sound)) return;
    if (options.preview !== true && Notification.isSupported()) {
      const notification = new Notification({
        title: 'Powermove', body: 'Your agent has finished. Your result is ready.', silent: true,
      });
      notification.on('click', () => {
        const window = BrowserWindow.fromWebContents(event.sender);
        if (!window || window.isDestroyed()) return;
        if (window.isMinimized()) window.restore();
        window.show(); window.focus();
      });
      notification.show();
    }
    if (process.platform === 'darwin' && options.sound !== 'None') {
      const soundPath = options.sound === 'Little Victory (Deep)'
        ? path.join(app.isPackaged ? process.resourcesPath : app.getAppPath(), app.isPackaged ? 'sounds' : 'resources/sounds', 'little-victory-deep.wav')
        : `/System/Library/Sounds/${options.sound}.aiff`;
      execFile('/usr/bin/afplay', [soundPath], () => {});
    }
  });
}
