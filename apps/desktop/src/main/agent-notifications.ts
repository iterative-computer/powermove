import { app, BrowserWindow, Notification, type IpcMain, type WebContents } from 'electron';
import path from 'node:path';
import { execFile } from 'node:child_process';
import { AGENT_SOUNDS } from '../shared/agent-notifications';
import { windowsScript } from './windows-system';

export function registerAgentNotifications(ipc: Pick<IpcMain, 'handle'>, ctx: {
  isTrustedSenderContents(sender: WebContents): boolean;
}): void {
  ipc.handle('agent:notification', (event, options) => {
    if (!ctx.isTrustedSenderContents(event.sender)) return;
    if (!options || !AGENT_SOUNDS.includes(options.sound)) return;
    if (options.preview !== true && Notification.isSupported()) {
      // A question names what the agent is asking; anything else is a finish.
      const question = typeof options.question === 'string'
        ? options.question.replace(/[\u0000-\u001f\u007f-\u009f]+/gu, ' ').replace(/\s+/gu, ' ').trim().slice(0, 240)
        : '';
      const notification = new Notification(question
        ? { title: 'Your agent has a question', body: question, silent: true }
        : { title: 'Powermove', body: 'Your agent has finished. Your result is ready.', silent: true });
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
    if (process.platform === 'win32' && options.sound !== 'None') {
      const soundPath = options.sound === 'Little Victory (Deep)'
        ? path.join(app.isPackaged ? process.resourcesPath : app.getAppPath(), app.isPackaged ? 'sounds' : 'resources/sounds', 'little-victory-deep.wav') : null;
      void windowsScript('if ($data) { $player = New-Object System.Media.SoundPlayer $data; try { $player.PlaySync() } finally { $player.Dispose() } } else { [System.Media.SystemSounds]::Asterisk.Play() }', soundPath).catch(() => undefined);
    }
  });
}
