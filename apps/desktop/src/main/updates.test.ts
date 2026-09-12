import { EventEmitter } from 'node:events';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  app: { isPackaged: true, getVersion: () => '1.0.0-beta.1', on: vi.fn() },
  show: vi.fn().mockResolvedValue({ response: 0 }),
  check: vi.fn().mockResolvedValue(null),
}));
vi.mock('electron', async () => {
  const { EventEmitter } = await import('node:events');
  return { app: mocks.app, autoUpdater: new EventEmitter(),
    dialog: { showMessageBox: mocks.show },
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
import { installUpdates } from './updates';

describe('macOS beta updates', () => {
  let item: { click(): void; label: string };
  const menu = { items: [{ submenu: { insert: (_: number, value: typeof item) => { item = value; } } }] };
  beforeEach(() => {
    vi.useFakeTimers(); vi.clearAllMocks();
    vi.spyOn(process, 'platform', 'get').mockReturnValue('darwin');
    mocks.app.isPackaged = true;
    (autoUpdater as EventEmitter).removeAllListeners();
    (nativeUpdater as EventEmitter).removeAllListeners();
  });
  afterEach(() => { vi.useRealTimers(); vi.restoreAllMocks(); });
  it('does not start an updater in development', async () => {
    mocks.app.isPackaged = false;
    installUpdates(menu as unknown as Menu);
    await vi.advanceTimersByTimeAsync(30_000);
    expect(mocks.check).not.toHaveBeenCalled();
  });
  it('checks beta releases without downgrades and coalesces checks', async () => {
    installUpdates(menu as unknown as Menu);
    expect(autoUpdater.allowPrerelease).toBe(true);
    expect(autoUpdater.allowDowngrade).toBe(false);
    await vi.advanceTimersByTimeAsync(30_000);
    item.click();
    expect(mocks.check).toHaveBeenCalledTimes(1);
  });
  it('waits for native staging and leaves quitting to the user', () => {
    installUpdates(menu as unknown as Menu);
    (autoUpdater as EventEmitter).emit('update-downloaded', {});
    expect(mocks.show).not.toHaveBeenCalled();
    nativeUpdater.emit('update-downloaded');
    expect(item.label).toContain('Quit to Install');
    expect(autoUpdater.autoInstallOnAppQuit).toBe(true);
    expect(autoUpdater.autoRunAppAfterInstall).toBe(false);
    expect(mocks.show).toHaveBeenCalledTimes(1);
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
