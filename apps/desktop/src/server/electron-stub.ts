/*
 * What `import ... from 'electron'` resolves to inside the remote host bundle.
 * Only the members main's modules actually touch at runtime are real; the
 * rest exist so the modules load, and throw if something reaches them.
 *
 * Dialogs cannot be shown on a headless box, so they are answered by the
 * connected browser (message boxes) or resolved to a fixed folder (saves).
 */
import { EventEmitter } from 'node:events';
import { createCipheriv, createDecipheriv, randomBytes } from 'node:crypto';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { homedir, tmpdir } from 'node:os';
import path from 'node:path';

import type { RemoteClient, RemoteWindow, WebIpcMain } from './clients';

export interface StubConfig {
  ipc: WebIpcMain;
  userData: string;
  appPath: string;
  resourcesPath: string;
  version: string;
  /** Where a "Save…" lands. */
  exportsDir: string;
  /** Ask the browser to decide a message box. Returns the chosen button index. */
  askMessageBox(window: RemoteWindow | null, options: MessageBoxOptions): Promise<number>;
  openExternal(url: string): void;
}

let config: StubConfig | null = null;
export function configureElectronStub(next: StubConfig): void { config = next; }
function cfg(): StubConfig {
  if (!config) throw new Error('electron stub used before configureElectronStub()');
  return config;
}

/* ── app ────────────────────────────────────────────────── */
class App extends EventEmitter {
  isPackaged = true;
  private name = 'Powermove';
  private readonly paths = new Map<string, string>();
  getPath(name: string): string {
    const override = this.paths.get(name);
    if (override) return override;
    switch (name) {
      case 'userData': return cfg().userData;
      case 'temp': return tmpdir();
      case 'home': return homedir();
      case 'downloads': return path.join(homedir(), 'Downloads');
      case 'logs': return path.join(cfg().userData, 'logs');
      default: return path.join(cfg().userData, name);
    }
  }
  setPath(name: string, value: string): void { this.paths.set(name, value); }
  getAppPath(): string { return cfg().appPath; }
  getName(): string { return this.name; }
  setName(name: string): void { this.name = name; }
  getVersion(): string { return cfg().version; }
  requestSingleInstanceLock(): boolean { return true; }
  whenReady(): Promise<void> { return Promise.resolve(); }
  quit(): void { this.emit('will-quit'); process.exit(0); }
  exit(code = 0): void { process.exit(code); }
  dock = { hide(): void {}, show(): void {} };
}
export const app = new App();

/* ── BrowserWindow ───────────────────────────────────────── */
export class BrowserWindow {
  static fromWebContents(contents: unknown): RemoteWindow | null { return cfg().ipc.fromWebContents(contents); }
  static getAllWindows(): RemoteWindow[] { return cfg().ipc.all().map((client) => client.window); }
  static getFocusedWindow(): RemoteWindow | null { return cfg().ipc.current()?.window ?? null; }
  constructor() { throw new Error('BrowserWindow cannot be created on the remote host.'); }
}

/* ── dialog ──────────────────────────────────────────────── */
export interface MessageBoxOptions {
  type?: string; message: string; detail?: string; buttons?: string[];
  defaultId?: number; cancelId?: number; noLink?: boolean;
}
export interface MessageBoxReturnValue { response: number; checkboxChecked: boolean }
export interface SaveDialogOptions { defaultPath?: string; filters?: Array<{ name: string; extensions: string[] }>; title?: string }
export interface SaveDialogReturnValue { canceled: boolean; filePath?: string }
export interface OpenDialogReturnValue { canceled: boolean; filePaths: string[] }

function uniquePath(dir: string, name: string): string {
  const ext = path.extname(name);
  const stem = name.slice(0, name.length - ext.length) || 'Untitled';
  let candidate = path.join(dir, `${stem}${ext}`);
  for (let index = 2; existsSync(candidate); index++) candidate = path.join(dir, `${stem}-${index}${ext}`);
  return candidate;
}

function asWindow(value: unknown): RemoteWindow | null {
  return value && typeof value === 'object' && 'webContents' in value ? (value as RemoteWindow) : null;
}

export const dialog = {
  async showSaveDialog(windowOrOptions: unknown, maybeOptions?: SaveDialogOptions): Promise<SaveDialogReturnValue> {
    const options = (maybeOptions ?? windowOrOptions ?? {}) as SaveDialogOptions;
    const dir = cfg().exportsDir;
    mkdirSync(dir, { recursive: true });
    const suggested = options.defaultPath ? path.basename(options.defaultPath) : 'Untitled';
    const extension = path.extname(suggested) ? '' : (options.filters?.[0]?.extensions[0] ? `.${options.filters[0].extensions[0]}` : '');
    return { canceled: false, filePath: uniquePath(dir, `${suggested}${extension}`) };
  },
  async showOpenDialog(): Promise<OpenDialogReturnValue> {
    // The browser picks files with <input type=file> and uploads them instead.
    return { canceled: true, filePaths: [] };
  },
  async showMessageBox(windowOrOptions: unknown, maybeOptions?: MessageBoxOptions): Promise<MessageBoxReturnValue> {
    const options = (maybeOptions ?? windowOrOptions) as MessageBoxOptions;
    const window = maybeOptions ? asWindow(windowOrOptions) : null;
    const response = await cfg().askMessageBox(window, options);
    return { response, checkboxChecked: false };
  },
  showErrorBox(title: string, content: string): void { console.error(`[dialog] ${title}: ${content}`); }
};

/* ── shell ───────────────────────────────────────────────── */
export const shell = {
  async openExternal(url: string): Promise<void> { cfg().openExternal(url); },
  showItemInFolder(fullPath: string): void { console.log(`[serve] reveal: ${fullPath}`); },
  async openPath(fullPath: string): Promise<string> { console.log(`[serve] open: ${fullPath}`); return ''; }
};

/* ── safeStorage: AES-256-GCM with a per-profile key file ─ */
function secretKey(): Buffer {
  const file = path.join(cfg().userData, 'secure-storage.key');
  try { return Buffer.from(readFileSync(file, 'utf8').trim(), 'base64'); } catch {
    const key = randomBytes(32);
    mkdirSync(path.dirname(file), { recursive: true });
    writeFileSync(file, key.toString('base64'), { mode: 0o600 });
    return key;
  }
}
export const safeStorage = {
  isEncryptionAvailable(): boolean { return true; },
  encryptString(plain: string): Buffer {
    const iv = randomBytes(12);
    const cipher = createCipheriv('aes-256-gcm', secretKey(), iv);
    const body = Buffer.concat([cipher.update(plain, 'utf8'), cipher.final()]);
    return Buffer.concat([iv, cipher.getAuthTag(), body]);
  },
  decryptString(encrypted: Buffer): string {
    const iv = encrypted.subarray(0, 12), tag = encrypted.subarray(12, 28), body = encrypted.subarray(28);
    const decipher = createDecipheriv('aes-256-gcm', secretKey(), iv);
    decipher.setAuthTag(tag);
    return Buffer.concat([decipher.update(body), decipher.final()]).toString('utf8');
  }
};

/* ── the rest: present so modules load ───────────────────── */
export class Notification extends EventEmitter {
  static isSupported(): boolean { return false; }
  show(): void {}
}
export const nativeTheme = { shouldUseDarkColors: false, themeSource: 'system' as 'system' | 'light' | 'dark' };
const unavailable = (name: string) => () => { throw new Error(`electron.${name} is not available on the remote host.`); };
export const Menu = { buildFromTemplate: unavailable('Menu.buildFromTemplate'), setApplicationMenu(): void {} };
export class MenuItem { constructor() { unavailable('MenuItem')(); } }
export const nativeImage = { createFromDataURL: unavailable('nativeImage'), createEmpty: unavailable('nativeImage') };
export const screen = { getDisplayMatching: unavailable('screen'), getDisplayNearestPoint: unavailable('screen'), getCursorScreenPoint: unavailable('screen') };
export const session = { defaultSession: {} };
export const protocol = { registerSchemesAsPrivileged(): void {}, handle(): void {} };
export const autoUpdater = new EventEmitter();
export const ipcMain = { handle: unavailable('ipcMain'), on: unavailable('ipcMain') };
export const webUtils = { getPathForFile: unavailable('webUtils') };
export type WebContents = RemoteClient;
export type IpcMain = WebIpcMain;
export type IpcMainEvent = unknown;
export type IpcMainInvokeEvent = unknown;
