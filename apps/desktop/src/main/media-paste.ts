import { BrowserWindow, clipboard, type IpcMain, type IpcMainInvokeEvent } from 'electron';
import { IPC } from '../shared/ipc';
import { FIGMA_CLIPBOARD_PREFIX, MAX_PAYLOAD } from '../shared/figma/doc';
import { userInput } from './user-input';

/** Request a native paste, without exposing clipboard contents through IPC. */
export function registerMediaPasteIpc(
  ipcMain: Pick<IpcMain, 'handle'>,
  ctx: {
    isTrustedSender(event: IpcMainInvokeEvent): boolean;
    windowFor?: (event: IpcMainInvokeEvent) => Pick<BrowserWindow, 'isDestroyed' | 'isFocused'> | null;
    userActed?: (event: IpcMainInvokeEvent) => Promise<boolean>;
    types?: () => Promise<string[]>;
    figmaPayload?: () => Promise<boolean>;
  },
): void {
  const windowFor = ctx.windowFor ?? ((event) => BrowserWindow.fromWebContents(event.sender));
  const acted = ctx.userActed ?? ((event) => userInput.acted(event.sender));
  const types = ctx.types ?? (async () => (await clipboard.read()).flatMap((item) => item.types));
  const figmaPayload =
    ctx.figmaPayload ??
    (async () => {
      for (const item of await clipboard.read()) {
        if (!item.types.includes('text/plain')) continue;
        const blob = await item.getType('text/plain');
        if (!('size' in blob) || blob.size > MAX_PAYLOAD) return false;
        return (await blob.text()).startsWith(FIGMA_CLIPBOARD_PREFIX);
      }
      return false;
    });
  ipcMain.handle(IPC.mediaPaste, async (event: IpcMainInvokeEvent): Promise<boolean> => {
    if (!ctx.isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    const window = windowFor(event);
    if (!window || window.isDestroyed() || !window.isFocused() || !(await acted(event))) return false;
    const formats = await types();
    const media =
      formats.some(
        (type) =>
          /^(image|video|audio)\//i.test(type) ||
          type === 'text/html' ||
          type === 'text/uri-list' ||
          type === 'application/x-moz-file' ||
          type === 'electron application/osclipboard;format="public.file-url"',
      ) ||
      (formats.includes('text/plain') && (await figmaPayload()));
    if (!media || window.isDestroyed() || !window.isFocused()) return false;
    event.sender.paste();
    return true;
  });
}
