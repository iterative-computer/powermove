import { EventEmitter } from 'node:events';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  app: { isPackaged: true, getVersion: () => '1.0.0-beta.1', on: vi.fn(), quit: vi.fn() },
  show: vi.fn().mockResolvedValue({ response: 0 }),
  check: vi.fn().mockResolvedValue(null),
  windows: [] as Array<{ isDestroyed(): boolean; webContents: { isDestroyed(): boolean; send: ReturnType<typeof vi.fn> } }>,
  handlers: new Map<string, (...args: unknown[]) => unknown>(),
}));
vi.mock('electron', async () => {
  const { EventEmitter } = await import('node:events');
  return { app: mocks.app, autoUpdater: new EventEmitter(),
    dialog: { showMessageBox: mocks.show },
    BrowserWindow: { getAllWindows: () => mocks.windows },
    ipcMain: { handle: (channel: string, fn: (...args: unknown[]) => unknown) => { mocks.handlers.set(channel, fn); } },
    Menu: { setApplicationMenu: vi.fn() },
    MenuItem: class { constructor(options: object) { Object.assign(this, options); } },
  };
});
vi.mock('electron-updater', async () => {
  const { EventEmitter } = await import('node:events');
  return { autoUpdater: Object.assign(new EventEmitter(), { checkForUpdates: mocks.check }) };
});
import { autoUpdater as nativeUpdater, type Menu } from 'electron';
import { autoUpdater } from 'electron-updater';
import { IPC } from '../shared/ipc';
import { installUpdates } from './updates';

function window() {
  return { isDestroyed: () => false, webContents: { isDestroyed: () => false, send: vi.fn() } };
}

describe('macOS updates', () => {
  let item: { click(): void; label: string };
  const menu = { items: [{ submenu: { insert: (_: number, value: typeof item) => { item = value; } } }] };
  beforeEach(() => {
    vi.useFakeTimers(); vi.clearAllMocks();
    vi.spyOn(process, 'platform', 'get').mockReturnValue('darwin');
    mocks.app.isPackaged = true;
    mocks.windows.length = 0;
    mocks.handlers.clear();
    (autoUpdater as EventEmitter).removeAllListeners();
    (nativeUpdater as EventEmitter).removeAllListeners();
    autoUpdater.autoRunAppAfterInstall = false;
  });
  afterEach(() => { vi.useRealTimers(); vi.restoreAllMocks(); });
  it('does not start an updater in development', async () => {
    mocks.app.isPackaged = false;
    installUpdates(menu as unknown as Menu);
    await vi.advanceTimersByTimeAsync(30_000);
    expect(mocks.check).not.toHaveBeenCalled();
    expect(mocks.handlers.size).toBe(0);
  });
  it('checks beta releases without downgrades and coalesces checks', async () => {
    installUpdates(menu as unknown as Menu);
    expect(autoUpdater.allowPrerelease).toBe(true);
    expect(autoUpdater.allowDowngrade).toBe(false);
    await vi.advanceTimersByTimeAsync(30_000);
    item.click();
    expect(mocks.check).toHaveBeenCalledTimes(1);
  });
  it('waits for native staging and falls back to a dialog without windows', () => {
    installUpdates(menu as unknown as Menu);
    (autoUpdater as EventEmitter).emit('update-downloaded', {});
    expect(mocks.show).not.toHaveBeenCalled();
    nativeUpdater.emit('update-downloaded');
    expect(item.label).toContain('Quit to Install');
    expect(autoUpdater.autoInstallOnAppQuit).toBe(true);
    expect(autoUpdater.autoRunAppAfterInstall).toBe(false);
    expect(mocks.show).toHaveBeenCalledTimes(1);
  });
  it('tells open windows about the staged version instead of a dialog', () => {
    const win = window();
    mocks.windows.push(win);
    installUpdates(menu as unknown as Menu);
    (autoUpdater as EventEmitter).emit('update-available', { version: '1.0.1' });
    expect(win.webContents.send).toHaveBeenLastCalledWith(IPC.updateChanged, expect.objectContaining({ status: 'downloading', version: '1.0.1' }));
    nativeUpdater.emit('update-downloaded');
    expect(win.webContents.send).toHaveBeenLastCalledWith(IPC.updateChanged, expect.objectContaining({ status: 'ready', version: '1.0.1', current: '1.0.0-beta.1' }));
    expect(mocks.show).not.toHaveBeenCalled();
    expect(mocks.handlers.get(IPC.updateStatus)!()).toEqual({ status: 'ready', version: '1.0.1', current: '1.0.0-beta.1' });
  });
  it('installs through a normal quit and relaunches only when an update is ready', () => {
    installUpdates(menu as unknown as Menu);
    mocks.handlers.get(IPC.updateInstall)!();
    expect(mocks.app.quit).not.toHaveBeenCalled();
    (autoUpdater as EventEmitter).emit('update-available', { version: '1.0.1' });
    nativeUpdater.emit('update-downloaded');
    mocks.handlers.get(IPC.updateInstall)!();
    expect(autoUpdater.autoRunAppAfterInstall).toBe(true);
    expect(mocks.app.quit).toHaveBeenCalledTimes(1);
  });
  it('keeps background failures quiet and allows a manual retry', async () => {
    vi.spyOn(console, 'error').mockImplementation(() => {});
    installUpdates(menu as unknown as Menu);
    await vi.advanceTimersByTimeAsync(30_000);
    autoUpdater.emit('error', new Error('offline'));
    expect(mocks.show).not.toHaveBeenCalled();
    item.click();
    (autoUpdater as EventEmitter).emit('update-not-available', {});
    expect(mocks.check).toHaveBeenCalledTimes(2);
    expect(mocks.show).toHaveBeenCalledWith(expect.objectContaining({ message: 'Powermove is up to date' }));
  });
});
