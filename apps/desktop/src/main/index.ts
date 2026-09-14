import { registerAgentNotifications } from './agent-notifications';
import { installUpdates } from './updates';
import { installTextContextMenu } from './text-context-menu';
import { registerRenderEncoder } from './render-encoder';
import {
  app,
  BrowserWindow,
  ipcMain,
  protocol,
  screen,
  session,
  shell,
  type WebContents
} from 'electron';
import { mkdir, readFile, readdir } from 'node:fs/promises';
import path from 'node:path';

import { IPC } from '../shared/ipc';
import { registerCaptureIpc } from './capture';
import { registerCodexIpc } from './codex';
import { recoverAllInterruptedExtensionTransactions } from './codex/change-history';
import { extensionAssetCorsHeaders, registerExtensionsIpc, serveExtensionAsset } from './extensions';
import { createExtensionRegistry } from './extensions/registry';
import { startExtensionWatcher } from './extensions/watcher';
import { registerLogIpc } from './log';
import { registerHapticsIpc } from './haptics';
import { registerContextMenuIpc } from './context-menu';
import { MediaProxyService, playbackConverter, registerMediaProxyIpc } from './media-proxy';
import { registerNativeEditIpc } from './native-edit';
import { installMenu, installRendererMenuShortcutRouting } from './menu';
import { registerSaveIpc } from './save';
import { ProjectFiles } from './project-files';
import { registerShellIpc } from './shell';
import { CONTENT_SECURITY_POLICY, SANDBOX_CONTENT_SECURITY_POLICY } from './security-policy';
import { createStore, installQuitFlush, registerStoreIpc } from './storage';
import { DARK_BACKGROUND, registerThemeIpc } from './theme';
import { backgroundTesting, backgroundWindowOptions } from './background-testing';
import { OnboardingFlow, onboardingCompleted, onboardingEnabled, persistOnboardingCompleted } from './onboarding';

const APP_ORIGIN = 'app://powermove';

// Electron derives the default userData directory from the application name.
// Set it before the first getPath('userData') call so development launches use
// the same Powermove profile as packaged builds (including generated effects).
app.setName('Powermove');

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
      // Dev renders from Vite's http origin, so generated app:// extension
      // modules need Chromium's normal CORS checks enabled for that boundary.
      corsEnabled: true,
      stream: true
    }
  }
]);

// e2e runs point this at a temp dir so tests never touch real user data.
const isBackgroundTest = backgroundTesting(process.env, app.getPath('userData'));
const userDataOverride = process.env['POWERMOVE_USER_DATA'];
if (userDataOverride && path.isAbsolute(userDataOverride)) {
  app.setPath('userData', userDataOverride);
}

const hasSingleInstanceLock = app.requestSingleInstanceLock();
let mainWindow: BrowserWindow | null = null;
let onboardingFlow: OnboardingFlow | null = null;
let startupInitialized = false;
let quitPrepared = () => false;
async function prepareEditorClose(window: BrowserWindow): Promise<void> {
  if (window.isDestroyed() || window.webContents.isDestroyed()
    || !isAllowedNavigation(window.webContents.getURL(), devRendererUrl)) return;
  // User decisions and large file saves must not be cut off by the recovery-flush timeout.
  const allowed = await window.webContents.executeJavaScript('window.PM?.prepareToClose?.() ?? true');
  if (!allowed) throw new Error('Close cancelled');
  await flushEditor(window);
}
async function flushEditor(window: BrowserWindow): Promise<void> {
  if (window.isDestroyed() || window.webContents.isDestroyed()
    || !isAllowedNavigation(window.webContents.getURL(),devRendererUrl)) return;
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    await Promise.race([window.webContents.executeJavaScript('window.PM?.flushProject?.()'),
      new Promise<never>((_resolve,reject) => {timer=setTimeout(() => reject(new Error('Editor save timed out')),5000);})]);
  } finally { clearTimeout(timer); }
}

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
const SANDBOX_CSP = SANDBOX_CONTENT_SECURITY_POLICY;

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
      if (requestedPath.startsWith('ext/')) {
        const asset = await serveExtensionAsset(requestedPath);
        if (asset === null) return errorResponse(404, 'Not found');
        const headers = responseHeaders('text/javascript; charset=utf-8');
        headers['Cache-Control'] = 'no-store';
        Object.assign(headers, extensionAssetCorsHeaders(devRendererUrl));
        return new Response(asset.body, { status: asset.status, headers });
      }
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

function createWindow(entrance = false, onEntranceReady?: () => void): BrowserWindow {
  const testOptions = backgroundWindowOptions(isBackgroundTest);
  const window = new BrowserWindow({
    ...testOptions,
    ...(entrance ? { show: false } : {}),
    width: 1440,
    height: 900,
    minWidth: 980,
    minHeight: 640,
    titleBarStyle: 'hiddenInset',
    trafficLightPosition: { x: 14, y: 15 },
    backgroundColor: DARK_BACKGROUND,
    webPreferences: {
      ...testOptions.webPreferences,
      preload: path.join(__dirname, '../preload/index.js'),
      contextIsolation: true,
      sandbox: true,
      nodeIntegration: false,
      autoplayPolicy: 'no-user-gesture-required'
    }
  });

  mainWindow = window;
  installRendererMenuShortcutRouting(window.webContents);
  installTextContextMenu(window.webContents);

  let closing = false, closePrepared = false;
  window.on('close', event => {
    if (closePrepared || quitPrepared() || window.webContents.isDestroyed()) return;
    event.preventDefault();
    if (closing) return;
    closing = true;
    void prepareEditorClose(window).then(() => {
      if (!window.isDestroyed()) { closePrepared=true;window.close();closePrepared=false; }
    }).catch(error => {
      if (error.message === 'Close cancelled') return;
      console.error('Could not save before closing',error);
      // Keep the editable document open when its durable save fails.
      if (!window.webContents.isDestroyed()) void window.webContents.executeJavaScript("window.PM?.toast?.('Could not save before closing. Your project is still open.',6000)").catch(() => undefined);
    }).finally(() => {closing=false;});
  });

  window.on('closed', () => {
    if (mainWindow === window) {
      mainWindow = null;
    }
  });

  if (entrance) {
    window.webContents.once('did-finish-load', () => {
      void window.webContents.executeJavaScript(`
        document.fonts.ready.then(() => {
          if (matchMedia('(prefers-reduced-motion: reduce)').matches) return;
          const home = document.querySelector('#projects-screen.on');
          const selector = home
            ? '#titlebar > *, .ps-sidebar, .ps-search, .ps-nav > *, .ps-sidefoot, .ps-main, .ps-top > *, .ps-view > *, .ps-grid > :not(.ps-empty), .ps-empty > *'
            : '#titlebar, #body > *, #status';
          const targets = [...document.querySelectorAll(selector)].filter(el => el.getClientRects().length);
          const animations = targets.map((el, index) => {
            const animation = el.animate([
              { opacity: 0, transform: 'translateY(18px)' },
              { opacity: 1, transform: 'translateY(0)' }
            ], { duration: 650, delay: (home ? 300 : 60) + index * 45, easing: 'cubic-bezier(0.16, 1, 0.3, 1)', fill: 'backwards' });
            animation.pause();
            return animation;
          });
          const play = () => animations.forEach(animation => {
            if (animation.playState === 'paused') animation.play();
          });
          window.addEventListener('focus', play, { once: true });
          setTimeout(play, 150);
        })
      `).catch(error => console.error('[onboarding] entrance failed', error)).finally(() => {
        if (!window.isDestroyed() && !isBackgroundTest) { window.show(); window.focus(); }
        onEntranceReady?.();
      });
    });
  }

  if (devRendererUrl) {
    void window.loadURL(devRendererUrl);
  } else {
    void window.loadURL(`${APP_ORIGIN}/`);
  }

  // Opt out with POWERMOVE_DEVTOOLS=0 (e2e: the DevTools window would otherwise
  // be the "first window" Playwright attaches to).
  if (!isBackgroundTest && !app.isPackaged && process.env['POWERMOVE_DEVTOOLS'] !== '0') {
    window.webContents.openDevTools({ mode: 'detach' });
  }

  return window;
}

if (!hasSingleInstanceLock) {
  app.quit();
} else {
  app.on('second-instance', () => {
    if (isBackgroundTest) return;
    // The listener is installed before async startup finishes. Do not let an
    // early second launch create the editor before the first-run gate decides
    // whether onboarding owns startup.
    if (!startupInitialized) return;
    if (onboardingFlow?.hasActiveWindow() || onboardingFlow?.isFirstRunPending()) {
      onboardingFlow.focus();
      return;
    }
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
    const proxyEncoder = app.isPackaged
      ? path.join(process.resourcesPath, 'encoder', 'ffmpeg')
      : path.join(app.getAppPath(), 'node_modules', 'ffmpeg-static', 'ffmpeg');
    const mediaProxies = new MediaProxyService(app.getPath('temp'), playbackConverter(proxyEncoder));
    registerAppProtocol();
    installPermissionHandlers();

    // Boot barrier: the legacy renderer reads PM.store synchronously while its
    // scripts load, so the store must be in memory before the window exists.
    const store = createStore(path.join(app.getPath('userData'), 'store'));
    await store.load();
    registerStoreIpc(ipcMain, store, { isTrustedSender });
    const quitBarrier = installQuitFlush(app, store, async () => {
      for (const window of BrowserWindow.getAllWindows()) await prepareEditorClose(window);
    });
    quitPrepared = quitBarrier.isPrepared;

    const userDir = path.join(app.getPath('userData'), 'extensions');
    const buildDir = path.join(app.getPath('userData'), 'extensions-build');
    const builtinResourcesDir = app.isPackaged
      ? path.join(process.resourcesPath, 'builtin-extensions')
      : path.resolve(app.getAppPath(), 'src/extensions');
    let refreshRestoredExtensions: ((ids: string[]) => Promise<void>) | undefined;
    // Extension boot must never prevent the window from appearing: a bad
    // directory or a slow compile degrades to "no user extensions" instead.
    try {
      await recoverAllInterruptedExtensionTransactions(app.getPath('userData'), userDir);
      await mkdir(userDir, { recursive: true });
      let builtinIds: string[] = [];
      try {
        builtinIds = (await readdir(builtinResourcesDir, { withFileTypes: true }))
          .filter((entry) => entry.isDirectory() && !entry.name.startsWith('.'))
          .map((entry) => entry.name)
          .sort();
      } catch {
        // No built-in directory (tests / partial checkouts): kernel still boots.
      }
      const extensionRegistry = createExtensionRegistry({
        store,
        userDir,
        buildDir,
        builtinIds,
        resourcesDir: builtinResourcesDir
      });
      registerExtensionsIpc(ipcMain, {
        registry: extensionRegistry,
        resourcesDir: builtinResourcesDir,
        isTrusted: isTrustedSender
      });
      refreshRestoredExtensions = async (ids) => {
        await extensionRegistry.refresh(ids);
        extensionRegistry.emitChanged({ ids, reason: 'reload' });
      };
      // Compile in the background; the renderer receives ext:changed when done.
      void extensionRegistry
        .refresh()
        .then(() => startExtensionWatcher({ userDir, buildDir, registry: extensionRegistry }))
        .catch((error) => console.error('[extensions] initial refresh failed', error));
    } catch (error) {
      console.error('[extensions] boot skipped', error);
    }

    const ctx = { isTrustedSender, isTrustedSenderContents };
    registerSaveIpc(ipcMain, { ...ctx, projects: new ProjectFiles(path.join(app.getPath('userData'), 'project-files.json')) });
    registerCaptureIpc(ipcMain, ctx);
    registerShellIpc(ipcMain, { ...ctx, attachmentCacheDirectory: path.join(app.getPath('userData'), 'Attachment Cache') });
    registerThemeIpc(ipcMain, ctx);
    registerHapticsIpc(ipcMain, ctx);
    registerContextMenuIpc(ipcMain, ctx);
    registerAgentNotifications(ipcMain, ctx);
    registerNativeEditIpc(ipcMain, ctx);
    registerLogIpc(ipcMain, ctx);
    registerMediaProxyIpc(ipcMain, mediaProxies, ctx);
    registerRenderEncoder(ipcMain,ctx);
    app.once('will-quit', () => { void mediaProxies.dispose(); });
    // API pack handed to the agent every autonomous run: the extension guide
    // plus the frozen kernel/shared/type contracts.
    const apiPackDir = app.isPackaged
      ? path.join(process.resourcesPath, 'api-pack')
      : app.getAppPath();
    const apiPackEntries: Array<[name: string, devPath: string]> = [
      ['EXTENSIONS.md', 'docs/EXTENSIONS.md'],
      ['BACKGROUND_TESTING.md', 'docs/background-testing.md'],
      ['api.ts', 'src/renderer/src/kernel/api.ts'],
      ['extensions.ts', 'src/shared/extensions.ts'],
      ['project.ts', 'src/renderer/src/core/types/project.ts'],
      ['commands.ts', 'src/renderer/src/core/types/commands.ts'],
      ['samples/media-browser/manifest.json', 'docs/samples/media-browser/manifest.json'],
      ['samples/media-browser/index.ts', 'docs/samples/media-browser/index.ts'],
      ['samples/media-browser/MediaBrowserPanel.svelte', 'docs/samples/media-browser/MediaBrowserPanel.svelte'],
      ['samples/media-browser/README.md', 'docs/samples/media-browser/README.md']
    ];
    const apiPackFiles = async (): Promise<Array<{ name: string; text: string }>> => {
      const files: Array<{ name: string; text: string }> = [];
      for (const [name, devPath] of apiPackEntries) {
        const file = app.isPackaged ? path.join(apiPackDir, name) : path.join(apiPackDir, devPath);
        try {
          // Dev and packaged builds hand the agent identical text: nothing
          // here may reveal the source checkout or other dev-only affordances.
          files.push({ name, text: await readFile(file, 'utf8') });
        } catch (error) {
          console.warn(`[api-pack] missing ${name}: ${String(error)}`);
        }
      }
      return files;
    };

    registerCodexIpc(ipcMain, {
      getWindow: () => mainWindow,
      userData: app.getPath('userData'),
      extensionsDir: userDir,
      apiPackFiles,
      isTrustedSender,
      codexBinaryPref: () => null, // a user-facing preference lands with the settings UI
      claudeBinaryPref: () => null,
      agentToolServerPath: app.isPackaged
        ? path.join(process.resourcesPath, 'agent-tools', 'mcp-server.mjs')
        : path.join(app.getAppPath(), 'src/main/agent-tools/mcp-server.mjs'),
      agentToolCommand: process.execPath,
      refreshExtensions: refreshRestoredExtensions,
      openExternal: async (url) => { await shell.openExternal(url); }
    });
    onboardingFlow = new OnboardingFlow(ipcMain, {
      appOrigin: (devRendererUrl ?? APP_ORIGIN).replace(/\/$/, ''),
      displayBounds: () => {
        if (mainWindow && !mainWindow.isDestroyed()) {
          return screen.getDisplayMatching(mainWindow.getBounds()).bounds;
        }
        return screen.getDisplayNearestPoint(screen.getCursorScreenPoint()).bounds;
      },
      backgroundTest: isBackgroundTest,
      userData: app.getPath('userData'),
      createEditor: () => {
        if (mainWindow && !mainWindow.isDestroyed()) {
          if (mainWindow.isMinimized()) mainWindow.restore();
          if (!isBackgroundTest) {
            mainWindow.show();
            mainWindow.focus();
          }
          return mainWindow;
        }
        return new Promise<BrowserWindow>((resolve, reject) => {
          const editor = createWindow(true, () => resolve(editor));
          editor.webContents.once('did-fail-load', (_event, _code, description) => {
            editor.destroy();
            reject(new Error(description));
          });
        });
      },
      secure: (window) => secureWebContents(window.webContents, devRendererUrl)
    });
    const menu = installMenu(() => mainWindow);
    if (!isBackgroundTest) installUpdates(menu);

    if (isBackgroundTest) app.dock?.hide();
    const completedOnboarding = await onboardingCompleted(app.getPath('userData'));
    if (onboardingEnabled(process.env, isBackgroundTest, completedOnboarding)) {
      // Record the first launch, even if the welcome is closed before Begin.
      await persistOnboardingCompleted(app.getPath('userData')).catch(error => {
        console.error('[onboarding] could not persist first launch', error);
      });
      onboardingFlow.start();
    } else {
      createWindow();
    }
    startupInitialized = true;

    app.on('activate', () => {
      if (isBackgroundTest) return;
      if (onboardingFlow?.hasActiveWindow() || onboardingFlow?.isFirstRunPending()) {
        onboardingFlow.focus();
        return;
      }
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
