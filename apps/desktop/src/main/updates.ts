import { app, autoUpdater as nativeUpdater, BrowserWindow, dialog, ipcMain, Menu, MenuItem } from 'electron';
import { autoUpdater } from 'electron-updater';

import { IPC, type AppUpdateState } from '../shared/ipc';

/** macOS stages updates with Squirrel and applies them on normal quit. Never
 * bypass the editor's before-quit save barrier with quitAndInstall(): the
 * renderer's Update button quits normally with autoRunAppAfterInstall on. */
export function installUpdates(menu: Menu): void {
  if (!app.isPackaged || process.platform !== 'darwin') return;
  autoUpdater.autoDownload = true;
  autoUpdater.autoInstallOnAppQuit = true;
  autoUpdater.autoRunAppAfterInstall = false;
  autoUpdater.allowPrerelease = app.getVersion().includes('-beta.');
  autoUpdater.allowDowngrade = false;

  const state: AppUpdateState = { status: 'idle', current: app.getVersion(), version: null };
  let manual = false;
  const show = (message: string, detail: string): void => {
    void dialog.showMessageBox({ type: 'info', title: 'Powermove Updates', message, detail }).catch(console.error);
  };
  const item = new MenuItem({ label: 'Check for Updates…', click: () => { void check(true); } });
  menu.items[0]?.submenu?.insert(1, item);
  Menu.setApplicationMenu(menu);

  function windows(): BrowserWindow[] {
    return BrowserWindow.getAllWindows().filter((window) => !window.isDestroyed() && !window.webContents.isDestroyed());
  }
  function set(next: Partial<AppUpdateState>): void {
    Object.assign(state, next);
    for (const window of windows()) window.webContents.send(IPC.updateChanged, { ...state });
  }

  async function check(interactive = false): Promise<void> {
    if (state.status === 'ready') {
      if (interactive) show('An update is ready', 'Quit Powermove normally to install it. Your session will be saved before quitting.');
      return;
    }
    if (state.status === 'checking' || state.status === 'downloading') {
      if (interactive) show('Checking or downloading an update', 'Powermove will let you know when the update is ready.');
      return;
    }
    manual = interactive;
    set({ status: 'checking' });
    try {
      await autoUpdater.checkForUpdates();
    } catch {
      // electron-updater emits the error event as well as rejecting; only
      // record the failure if that event did not already move the state on.
      if ((state.status as AppUpdateState['status']) === 'checking') set({ status: 'error' });
    }
  }
  autoUpdater.on('update-available', (info: { version?: string }) => {
    set({ status: 'downloading', version: info?.version ?? null });
  });
  autoUpdater.on('update-not-available', () => {
    set({ status: 'idle', version: null });
    if (manual) show('Powermove is up to date', `You’re running ${app.getVersion()}.`);
    manual = false;
  });
  autoUpdater.on('error', (error) => {
    set({ status: 'error' });
    console.error('[updates]', error);
    if (manual) show('Could not check for updates', 'Please try again later. You can continue using Powermove.');
    manual = false;
  });
  // The electron-updater event precedes Squirrel staging. Only announce the
  // update after the native updater confirms it can install on exit.
  nativeUpdater.on('update-downloaded', () => {
    manual = false;
    item.label = 'Update Ready — Quit to Install';
    set({ status: 'ready' });
    // Windows show the in-app notice; without one, fall back to a dialog.
    if (windows().length === 0) {
      show('A Powermove update is ready', 'It will install when you quit Powermove. Keep working and quit whenever you’re ready.');
    }
  });

  ipcMain.handle(IPC.updateStatus, () => ({ ...state }));
  ipcMain.handle(IPC.updateCheck, () => check(false));
  ipcMain.handle(IPC.updateInstall, () => {
    if (state.status !== 'ready') return;
    // A normal quit runs the save barrier; Squirrel installs on exit and
    // relaunches into the new version.
    autoUpdater.autoRunAppAfterInstall = true;
    app.quit();
  });

  const startup = setTimeout(() => { void check(); }, 30_000);
  const interval = setInterval(() => { void check(); }, 4 * 60 * 60 * 1000);
  startup.unref();
  interval.unref();
  app.on('will-quit', () => { clearTimeout(startup); clearInterval(interval); });
}
