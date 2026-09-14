import { BrowserWindow, Menu, type IpcMain, type IpcMainInvokeEvent, type MenuItemConstructorOptions, type WebContents } from 'electron';

import { IPC, type NativeMenuItem, type NativeMenuRequest } from '../shared/ipc';

export type NativeMenuPoint = { x?: number; y?: number };

export interface ContextMenuIpcContext {
  isTrustedSenderContents(sender: WebContents): boolean;
  /** Test seam. Defaults to Electron's Menu.popup; resolves with the clicked id. */
  popup?: (template: MenuItemConstructorOptions[], window: BrowserWindow | undefined, at: NativeMenuPoint) => Promise<void>;
}

const MAX_ITEMS = 200;

/** Renderer payloads are untrusted: keep only well-formed rows. */
export function sanitizeNativeMenu(items: unknown): NativeMenuItem[] {
  if (!Array.isArray(items)) return [];
  const out: NativeMenuItem[] = [];
  for (const raw of items.slice(0, MAX_ITEMS)) {
    if (!raw || typeof raw !== 'object') continue;
    const item = raw as Record<string, unknown>;
    if (item.type === 'separator') { out.push({ type: 'separator' }); continue; }
    if (item.type !== 'normal' && item.type !== 'checkbox' && item.type !== 'header') continue;
    if (typeof item.id !== 'string' || typeof item.label !== 'string') continue;
    out.push({
      type: item.type,
      id: item.id,
      label: item.label.slice(0, 200),
      enabled: item.enabled !== false,
      checked: item.checked === true,
      accelerator: typeof item.accelerator === 'string' && item.accelerator ? item.accelerator : undefined
    });
  }
  return out;
}

/** Menu rows → an Electron template; `onClick` receives the chosen id. */
export function nativeMenuTemplate(items: NativeMenuItem[], onClick: (id: string) => void): MenuItemConstructorOptions[] {
  return items.map((item): MenuItemConstructorOptions => {
    if (item.type === 'separator') return { type: 'separator' };
    /* NSMenu has no section titles; a disabled row reads the same way. */
    if (item.type === 'header') return { label: item.label, enabled: false };
    return {
      type: item.type,
      label: item.label,
      enabled: item.enabled !== false,
      checked: item.type === 'checkbox' ? !!item.checked : undefined,
      accelerator: item.accelerator,
      /* Key equivalents are hints; the renderer already owns the shortcuts. */
      registerAccelerator: false,
      click: () => onClick(item.id)
    };
  });
}

function popupWithElectron(template: MenuItemConstructorOptions[], window: BrowserWindow | undefined, at: NativeMenuPoint): Promise<void> {
  return new Promise((resolve) => {
    Menu.buildFromTemplate(template).popup({
      window,
      x: at.x != null ? Math.round(at.x) : undefined,
      y: at.y != null ? Math.round(at.y) : undefined,
      /* Fires after the click handler, so the chosen id is already recorded. */
      callback: () => resolve()
    });
  });
}

export function registerContextMenuIpc(ipcMain: Pick<IpcMain, 'handle'>, ctx: ContextMenuIpcContext): void {
  const popup = ctx.popup ?? popupWithElectron;
  ipcMain.handle(IPC.menuPopup, async (event: IpcMainInvokeEvent, request: NativeMenuRequest): Promise<string | null> => {
    if (!ctx.isTrustedSenderContents(event.sender)) throw new Error('Unauthorized IPC sender');
    const items = sanitizeNativeMenu(request?.items);
    if (!items.some((item) => item.type === 'normal' || item.type === 'checkbox')) return null;
    const window = BrowserWindow.fromWebContents(event.sender) ?? undefined;
    const finite = (value: unknown) => (typeof value === 'number' && Number.isFinite(value) ? value : undefined);
    let chosen: string | null = null;
    await popup(nativeMenuTemplate(items, (id) => { chosen = id; }), window, { x: finite(request?.x), y: finite(request?.y) });
    return chosen;
  });
}
