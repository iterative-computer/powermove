import type { IpcMain, IpcMainInvokeEvent, WebContents } from 'electron';
import { StudioHostManager } from './manager';
import { STUDIO_NATIVE_IPC, STUDIO_MAX_BINARY, type StudioNativeCommand } from '../../shared/studio-native';

/** No paths, executable names, or OS handles are accepted from the page. */
export function registerStudioNativeIpc(ipc: Pick<IpcMain, 'handle' | 'removeHandler'>, executable: string | null,
  trusted: (event: IpcMainInvokeEvent) => boolean): () => void {
  const sessions = new Map<number, Map<string, StudioHostManager>>();
  const watched = new Set<number>();
  const check = (event: IpcMainInvokeEvent, key: unknown): string => {
    if (!trusted(event)) throw new Error('Unauthorized Studio IPC sender');
    if (typeof key !== 'string' || !/^[a-zA-Z0-9:_-]{1,128}$/.test(key)) throw new Error('Invalid Studio session');
    return key;
  };
  const releaseAll = (id: number) => { for (const manager of sessions.get(id)?.values() ?? []) manager.dispose(); sessions.delete(id); };
  const watch = (contents: WebContents) => {
    if (watched.has(contents.id)) return;
    watched.add(contents.id);
    contents.on('render-process-gone', () => releaseAll(contents.id));
    contents.on('did-start-navigation', (_event, _url, _inPlace, mainFrame) => { if (mainFrame) releaseAll(contents.id); });
    contents.once('destroyed', () => { releaseAll(contents.id); watched.delete(contents.id); });
  };
  ipc.handle(STUDIO_NATIVE_IPC.request, async (event, key: unknown, command: StudioNativeCommand, binary?: Uint8Array) => {
    const id = check(event, key);
    if (binary !== undefined && (!(binary instanceof Uint8Array) || binary.byteLength > STUDIO_MAX_BINARY)) throw new Error('Invalid Studio binary payload');
    let owner = sessions.get(event.sender.id);
    if (!owner) { owner = new Map(); sessions.set(event.sender.id, owner); watch(event.sender); }
    let manager = owner.get(id);
    if (!manager) {
      // Bound the native memory budget; the backend releases idle sessions.
      if (owner.size >= 2) throw new Error('Too many active Studio sessions');
      manager = new StudioHostManager({ executable }); owner.set(id, manager);
    }
    return manager.request(command, binary);
  });
  ipc.handle(STUDIO_NATIVE_IPC.release, (event, key: unknown) => { const id = check(event, key); const owner = sessions.get(event.sender.id); owner?.get(id)?.dispose(); owner?.delete(id); });
  ipc.handle(STUDIO_NATIVE_IPC.status, event => {
    if (!trusted(event)) throw new Error('Unauthorized Studio IPC sender');
    const manager = sessions.get(event.sender.id)?.values().next().value;
    return manager?.status() ?? { available: !!executable, running: false, generation: 0, failure: executable ? null : 'Native Studio is not installed' };
  });
  return () => { for (const id of sessions.keys()) releaseAll(id); for (const channel of Object.values(STUDIO_NATIVE_IPC)) ipc.removeHandler(channel); };
}
