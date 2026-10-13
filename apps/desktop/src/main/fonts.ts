import type { IpcMain, IpcMainInvokeEvent } from 'electron';
import { fontFamilies, fontRegistryVersion } from '@powermove/macos-haptics';
import { IPC } from '../shared/ipc';

export function registerFontsIpc(ipc: Pick<IpcMain, 'handle'>, ctx: {
  isTrustedSenderContents(sender: IpcMainInvokeEvent['sender']): boolean;
}) {
  let cached: { version: number; families: string[] } | null = null;
  ipc.handle(IPC.fontFamilies, event => {
    if (!ctx.isTrustedSenderContents(event.sender)) return null;
    const version = fontRegistryVersion();
    if (version !== null && cached?.version === version) return cached.families.slice();
    const families = fontFamilies();
    // Retain a successful snapshot only when the registry stayed unchanged
    // throughout enumeration. Failed/empty reads and older native addons keep
    // their existing fresh-read behavior and can recover on the next request.
    cached = version !== null && families?.length && fontRegistryVersion() === version
      ? { version, families: families.slice() } : null;
    return families;
  });
}
