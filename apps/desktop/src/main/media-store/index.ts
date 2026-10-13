/*
 * Installs the host media store in Electron main (docs/engine/media-store.md).
 * The store exists in both engine modes: with "Use New Engine" on it is the
 * source of project media bytes; with it off the renderer keeps using
 * IndexedDB and reads the store only for media that was imported with the
 * flag on, so switching engines loses nothing.
 */
import path from 'node:path';

import type { IpcMain, IpcMainEvent, IpcMainInvokeEvent } from 'electron';

import { WorkerPool } from './hasher';
import { registerMediaStoreIpc } from './ipc';
import { MediaStoreService } from './service';
import { STORE_DIRECTORY } from './store';

export { MediaStoreService } from './service';
export { STORE_DIRECTORY } from './store';

export type InstallMediaStoreOptions = {
  ipc: Pick<IpcMain, 'handle' | 'on'>;
  isTrustedSender(event: IpcMainInvokeEvent | IpcMainEvent): boolean;
  userData: string;
  /** Directory holding the built `media-hash-worker.js` (main's `__dirname`). */
  mainDirectory: string;
  enabled: boolean;
};

export function installMediaStore(options: InstallMediaStoreOptions): { service: MediaStoreService; dispose(): Promise<void> } {
  const executor = new WorkerPool(path.join(options.mainDirectory, 'media-hash-worker.js'));
  const service = new MediaStoreService({
    root: path.join(options.userData, STORE_DIRECTORY),
    userData: options.userData,
    executor,
    enabled: options.enabled
  });
  void service.store.init().catch((error: unknown) => console.warn('[media-store] could not prepare the media store', error));
  registerMediaStoreIpc(options.ipc, service, { isTrustedSender: options.isTrustedSender });
  return { service, dispose: () => executor.close() };
}
