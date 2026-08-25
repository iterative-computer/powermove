import {
  app,
  BrowserWindow,
  ipcMain,
  protocol,
  session,
  shell,
  type WebContents
} from 'electron';
import { readFile } from 'node:fs/promises';
import path from 'node:path';

import { IPC } from '../shared/ipc';
import { registerCaptureIpc } from './capture';
import { registerCodexIpc } from './codex';
import { registerLogIpc } from './log';
import { installMenu } from './menu';
import { registerSaveIpc } from './save';
import { registerShellIpc } from './shell';
import { createStore, installQuitFlush, registerStoreIpc } from './storage';
import { registerThemeIpc } from './theme';

const APP_ORIGIN = 'app://powermove';
const CONTENT_SECURITY_POLICY =
  "default-src 'none'; script-src 'self' 'unsafe-eval'; style-src 'self' 'unsafe-inline'; img-src 'self' blob: data:; media-src 'self' blob:; font-src 'self'; connect-src 'self' blob:; worker-src 'self' blob:; frame-src 'self' about: blob:";
// Served with X-Content-Type-Options: nosniff, so anything not listed here is
// rejected by <video>/<audio>/WebAssembly rather than sniffed.
const MIME_TYPES: Readonly<Record<string, string>> = {
  '.aac': 'audio/aac',
  '.avif': 'image/avif',
  '.css': 'text/css; charset=utf-8',
  '.flac': 'audio/flac',
  '.gif': 'image/gif',
  '.html': 'text/html; charset=utf-8',
  '.ico': 'image/x-icon',
  '.jpeg': 'image/jpeg',
  '.jpg': 'image/jpeg',
  '.js': 'text/javascript; charset=utf-8',
  '.json': 'application/json; charset=utf-8',
  '.m4a': 'audio/mp4',
  '.m4v': 'video/mp4',
  '.map': 'application/json; charset=utf-8',
  '.mjs': 'text/javascript; charset=utf-8',
  '.mov': 'video/quicktime',
  '.mp3': 'audio/mpeg',
  '.mp4': 'video/mp4',
  '.ogg': 'audio/ogg',
  '.png': 'image/png',
  '.svg': 'image/svg+xml',
  '.ttf': 'font/ttf',
  '.wasm': 'application/wasm',
  '.wav': 'audio/wav',
  '.webm': 'video/webm',
  '.webp': 'image/webp',
  '.woff': 'font/woff',
  '.woff2': 'font/woff2'
};

// Media imported by the user stays on blob: URLs (IndexedDB-backed), so the
// app:// handler never needs Range/206 support for the bundle it serves.

protocol.registerSchemesAsPrivileged([
  {
    scheme: 'app',
    privileges: {
      standard: true,
      secure: true,
      supportFetchAPI: true,
      stream: true
    }
  }
]);

// e2e runs point this at a temp dir so tests never touch real user data.
const userDataOverride = process.env['POWERMOVE_USER_DATA'];
if (userDataOverride && path.isAbsolute(userDataOverride)) {
  app.setPath('userData', userDataOverride);
}

const hasSingleInstanceLock = app.requestSingleInstanceLock();
let mainWindow: BrowserWindow | null = null;

function responseHeaders(contentType: string): Record<string, string> {
  return {
    'Content-Security-Policy': CONTENT_SECURITY_POLICY,
    'Content-Type': contentType,
    'X-Content-Type-Options': 'nosniff'
  };
}

function errorResponse(status: number, message: string): Response {
  return new Response(message, {
    status,
    headers: responseHeaders('text/plain; charset=utf-8')
  });
}

// The renderer is the Vite-built bundle in out/renderer (the legacy app runs
// from src/renderer/src/legacy/* modules since Phase 4).

// The generated-script sandbox document is served with its own policy: Chromium
// inherits the parent CSP into srcdoc/blob frames, so it must be a real URL.
const SANDBOX_PATH = 'host/sandbox.html';
const SANDBOX_CSP =
  "default-src 'none'; script-src app://powermove/host/sandbox.js 'unsafe-eval'; worker-src blob:; connect-src 'none'";

function registerAppProtocol(): void {
  const rendererRoot = path.resolve(__dirname, '../renderer');

  protocol.handle('app', async (request) => {
    try {
      const requestUrl = new URL(request.url);
      if (requestUrl.host !== 'powermove') {
        return errorResponse(404, 'Not found');
      }

      const pathname = decodeURIComponent(requestUrl.pathname);
      const requestedPath = pathname === '/' ? 'index.html' : pathname.replace(/^\/+/, '');
      const filePath = path.resolve(rendererRoot, requestedPath);
      const relativePath = path.relative(rendererRoot, filePath);

      if (
        relativePath === '..' ||
        relativePath.startsWith(`..${path.sep}`) ||
        path.isAbsolute(relativePath)
      ) {
        return errorResponse(403, 'Forbidden');
      }

      const contents = await readFile(filePath);
      const contentType = MIME_TYPES[path.extname(filePath).toLowerCase()] ?? 'application/octet-stream';
      const headers = responseHeaders(contentType);
      if (relativePath === SANDBOX_PATH) headers['Content-Security-Policy'] = SANDBOX_CSP;
      return new Response(contents, { headers });
    } catch {
      return errorResponse(404, 'Not found');
    }
  });
}

function isAllowedNavigation(targetUrl: string, devRendererUrl: string | undefined): boolean {
  try {
    const target = new URL(targetUrl);
    if (target.protocol === 'app:' && target.host === 'powermove') {
      return true;
    }

    return devRendererUrl !== undefined && target.origin === new URL(devRendererUrl).origin;
  } catch {
    return false;
  }
}

function secureWebContents(webContents: WebContents, devRendererUrl: string | undefined): void {
  const guard = (event: { preventDefault(): void }, targetUrl: string): void => {
    if (!isAllowedNavigation(targetUrl, devRendererUrl)) {
      event.preventDefault();
    }
  };
  webContents.on('will-navigate', guard);
  // Subframe navigations (frame-src allows about:/blob:) never fire
  // will-navigate on the host, so they need their own hook.
  webContents.on('will-frame-navigate', (event) => guard(event, event.url));
  webContents.on('will-redirect', guard);

  webContents.setWindowOpenHandler(({ url }) => {
    try {
      const target = new URL(url);
      if (target.protocol === 'https:' || target.protocol === 'http:') {
        void shell.openExternal(target.toString());
      }
    } catch {
      // Malformed URLs are denied below.
    }

    return { action: 'deny' };
  });
}

// Default-deny; only the app's main frame may enumerate local fonts
// (PM.Fonts is fed from queryLocalFonts() at boot — see docs/phase0-decisions.md).
const GRANTED_PERMISSIONS = new Set(['local-fonts']);

function isAppMainFrame(webContents: WebContents | null, requestingUrl: string | undefined): boolean {
  if (!webContents || webContents.isDestroyed()) return false;
  const url = requestingUrl ?? webContents.getURL();
  return isAllowedNavigation(url, devRendererUrl);
}

function installPermissionHandlers(): void {
  session.defaultSession.setPermissionRequestHandler((webContents, permission, callback, details) => {
    callback(GRANTED_PERMISSIONS.has(permission) && isAppMainFrame(webContents, details.requestingUrl));
  });
  session.defaultSession.setPermissionCheckHandler((webContents, permission, requestingOrigin) => {
    return GRANTED_PERMISSIONS.has(permission) && isAppMainFrame(webContents, requestingOrigin);
  });
}

// A packaged build must never load a renderer URL inherited from the environment.
const devRendererUrl = app.isPackaged ? undefined : process.env['ELECTRON_RENDERER_URL'];

// Only the app document's main frame may use privileged IPC (never the
// sandbox iframe, never a stray WebContents).
function isTrustedSender(event: { sender: WebContents; senderFrame: Electron.WebFrameMain | null }): boolean {
  const win = BrowserWindow.fromWebContents(event.sender);
  if (!win || win.isDestroyed() || event.senderFrame === null) {
    return false;
  }
  return (
    event.senderFrame === win.webContents.mainFrame &&
    isAllowedNavigation(event.senderFrame.url, devRendererUrl)
  );
}

function isTrustedSenderContents(sender: WebContents): boolean {
  const win = BrowserWindow.fromWebContents(sender);
  return !!win && !win.isDestroyed() && isAllowedNavigation(sender.getURL(), devRendererUrl);
}

function createWindow(): BrowserWindow {
  const window = new BrowserWindow({
    width: 1440,
    height: 900,
    minWidth: 980,
    minHeight: 640,
    titleBarStyle: 'hiddenInset',
    trafficLightPosition: { x: 14, y: 12 },
    backgroundColor: '#0b0b0c',
    webPreferences: {
      preload: path.join(__dirname, '../preload/index.js'),
      contextIsolation: true,
      sandbox: true,
      nodeIntegration: false,
      autoplayPolicy: 'no-user-gesture-required'
    }
  });

  mainWindow = window;

  window.on('closed', () => {
    if (mainWindow === window) {
      mainWindow = null;
    }
  });

  if (devRendererUrl) {
    void window.loadURL(devRendererUrl);
  } else {
    void window.loadURL(`${APP_ORIGIN}/`);
  }

  // Opt out with POWERMOVE_DEVTOOLS=0 (e2e: the DevTools window would otherwise
  // be the "first window" Playwright attaches to).
  if (!app.isPackaged && process.env['POWERMOVE_DEVTOOLS'] !== '0') {
    window.webContents.openDevTools({ mode: 'detach' });
  }

  return window;
}

if (!hasSingleInstanceLock) {
  app.quit();
} else {
  app.setName('Powermove');

  app.on('second-instance', () => {
    if (mainWindow === null) {
      createWindow();
      return;
    }
    if (mainWindow.isMinimized()) {
      mainWindow.restore();
    }
    mainWindow.show();
    mainWindow.focus();
  });

  // Every WebContents (not just the main window's) gets the navigation guards.
  app.on('web-contents-created', (_event, contents) => {
    secureWebContents(contents, devRendererUrl);
  });

  ipcMain.handle(IPC.ping, (event) => {
    if (!isTrustedSender(event)) {
      throw new Error('Unauthorized IPC sender');
    }
    return 'pong';
  });

  void app.whenReady().then(async () => {
    registerAppProtocol();
    installPermissionHandlers();

    // Boot barrier: the legacy renderer reads PM.store synchronously while its
    // scripts load, so the store must be in memory before the window exists.
    const store = createStore(path.join(app.getPath('userData'), 'store'));
    await store.load();
    registerStoreIpc(ipcMain, store, { isTrustedSender });
    installQuitFlush(app, store);

    const ctx = { isTrustedSender, isTrustedSenderContents };
    registerSaveIpc(ipcMain, ctx);
    registerCaptureIpc(ipcMain, ctx);
    registerShellIpc(ipcMain, ctx);
    registerThemeIpc(ipcMain, ctx);
    registerLogIpc(ipcMain, ctx);
    registerCodexIpc(ipcMain, {
      getWindow: () => mainWindow,
      userData: app.getPath('userData'),
      isTrustedSender,
      codexBinaryPref: () => null // a user-facing preference lands with the settings UI
    });
    installMenu(() => mainWindow);

    createWindow();

    app.on('activate', () => {
      if (BrowserWindow.getAllWindows().length === 0) {
        createWindow();
      }
    });
  });
}

app.on('window-all-closed', () => {
  if (process.platform !== 'darwin') {
    app.quit();
  }
});
