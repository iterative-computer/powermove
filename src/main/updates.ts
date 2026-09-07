import { app, autoUpdater as nativeUpdater, dialog, Menu, MenuItem } from 'electron';
import { autoUpdater } from 'electron-updater';

/** macOS stages updates with Squirrel and applies them on normal quit. Never
 * bypass the editor's before-quit save barrier with quitAndInstall(). */
export function installUpdates(menu: Menu): void {
  if (!app.isPackaged || process.platform !== 'darwin') return;
  autoUpdater.autoDownload = true;
  autoUpdater.autoInstallOnAppQuit = true;
  autoUpdater.autoRunAppAfterInstall = false;
  autoUpdater.allowPrerelease = app.getVersion().includes('-beta.');
  autoUpdater.allowDowngrade = false;

  let busy = false;
  let ready = false;
  let manual = false;
  const show = (message: string, detail: string): void => {
    void dialog.showMessageBox({ type: 'info', title: 'Powermove Updates', message, detail }).catch(console.error);
  };
  const item = new MenuItem({ label: 'Check for Updates…', click: () => { void check(true); } });
  menu.items[0]?.submenu?.insert(1, item);
  Menu.setApplicationMenu(menu);

  async function check(interactive = false): Promise<void> {
    if (ready) {
      if (interactive) show('An update is ready', 'Quit Powermove normally to install it. Your session will be saved before quitting.');
      return;
    }
    if (busy) {
      if (interactive) show('Checking or downloading an update', 'Powermove will let you know when the update is ready.');
      return;
    }
    busy = true;
    manual = interactive;
    try {
      await autoUpdater.checkForUpdates();
    } catch {
      // electron-updater emits the error event as well as rejecting.
      busy = false;
    }
  }
  autoUpdater.on('update-not-available', () => {
    busy = false;
    if (manual) show('Powermove is up to date', `You’re running ${app.getVersion()}.`);
    manual = false;
  });
  autoUpdater.on('error', (error) => {
    busy = false;
    console.error('[updates]', error);
    if (manual) show('Could not check for updates', 'Please try again later. You can continue using Powermove.');
    manual = false;
  });
  // The electron-updater event precedes Squirrel staging. Only announce the
  // update after the native updater confirms it can install on exit.
  nativeUpdater.on('update-downloaded', () => {
    busy = false;
    ready = true;
    manual = false;
    item.label = 'Update Ready — Quit to Install';
    show('A Powermove update is ready', 'It will install when you quit Powermove. Keep working and quit whenever you’re ready.');
  });
  const startup = setTimeout(() => { void check(); }, 30_000);
  const interval = setInterval(() => { void check(); }, 4 * 60 * 60 * 1000);
  startup.unref();
  interval.unref();
  app.on('will-quit', () => { clearTimeout(startup); clearInterval(interval); });
}
