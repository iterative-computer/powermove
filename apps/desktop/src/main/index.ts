import { registerFontsIpc } from './fonts';
import { registerAgentNotifications } from './agent-notifications';
import { installUpdates } from './updates';
import { installTextContextMenu } from './text-context-menu';
import { installPermissionHandlers } from './permissions';
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

import {
  IPC,
  PROJECT_ID,
  type WindowClaimResult,
  type WindowInitialProject,
  type WindowOpenResult
} from '../shared/ipc';
import { registerCaptureIpc } from './capture';
import { registerCodexIpc } from './codex';
import { configureRuntimeUpdates } from './runtime-updates';
import { recoverAllInterruptedExtensionTransactions } from './codex/change-history';
import { extensionAssetCorsHeaders, registerExtensionsIpc, serveExtensionAsset } from './extensions';
import { createExtensionRegistry } from './extensions/registry';
import { startExtensionWatcher } from './extensions/watcher';
import { registerLogIpc } from './log';
import { MIME_TYPES } from './mime';
import { registerHapticsIpc } from './haptics';
import { registerContextMenuIpc } from './context-menu';
import { registerCloudMediaIpc } from './cloud-media';
import { registerConfirmIpc } from './native-confirm';
import { MediaProxyService, playbackConverter, previewConverter, imageSequenceConverter, stillImageConverter, registerMediaProxyIpc } from './media-proxy';
import { registerNativeEditIpc } from './native-edit';
import { installMenu, installRendererMenuShortcutRouting } from './menu';
import { openProjectForWindow, registerSaveIpc } from './save';
import { ProjectFiles } from './project-files';
import { registerShellIpc } from './shell';
import { CONTENT_SECURITY_POLICY, SANDBOX_CONTENT_SECURITY_POLICY } from './security-policy';
import { createStore, installQuitFlush, registerStoreIpc, type Store } from './storage';
import { registerThemeIpc } from './theme';
import { backgroundTesting, backgroundWindowOptions } from './background-testing';
import { EditorWindows, restorableProjects } from './windows';
import { isPanelPopoutRequest } from './panel-popout';
import { OnboardingFlow, onboardingCompleted, onboardingEnabled, persistOnboardingCompleted } from './onboarding';

const APP_ORIGIN = 'app://powermove';

// Electron derives the default userData directory from the application name.
// Set it before the first getPath('userData') call so development launches use
// the same Powermove profile as packaged builds (including generated effects).
app.setName('Powermove');

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
/* One project per window, any number of windows. `editors` is the whole of the
   app's window state; nothing below may assume a single "main" window. */
const editors = new EditorWindows<BrowserWindow>();
/** The window a menu command or a Dock activation belongs to. */
const currentEditor = (): BrowserWindow | null => {
  const focused = BrowserWindow.getFocusedWindow();
  return focused && editors.has(focused) ? focused : editors.mostRecent();
};
/* Set once the store exists; before that a window cannot have a project yet. */
let persistOpenWindows: () => void = () => {};
let onboardingFlow: OnboardingFlow | null = null;
let startupInitialized = false;
let projects: ProjectFiles | null = null;
const pendingOpenFiles: string[] = [];
const recentOpenFiles = new Map<string, number>();
let quitPrepared = () => false;

function queueOrOpen(filePath: string): void {
  const mainWindow = currentEditor();
  const normalized = path.resolve(filePath);
  const now = Date.now();
  const lastOpen = recentOpenFiles.get(normalized);
  if (lastOpen !== undefined && now - lastOpen < 1_000) return;
  recentOpenFiles.set(normalized, now);
  for (const [candidate, timestamp] of recentOpenFiles) {
    if (now - timestamp >= 1_000) recentOpenFiles.delete(candidate);
  }
  if (!startupInitialized || !mainWindow || mainWindow.isDestroyed()
    || mainWindow.webContents.isDestroyed() || mainWindow.webContents.isLoading() || !projects) {
    if (!pendingOpenFiles.includes(normalized)) pendingOpenFiles.push(normalized);
    return;
  }
  const target = mainWindow.webContents;
  void openProjectForWindow({ projects }, target, normalized).then(result => {
    if (!target.isDestroyed()) target.send(IPC.projectOpenExternal, result);
  });
}

function drainPendingOpenFiles(): void {
  const mainWindow = currentEditor();
  if (!startupInitialized || !mainWindow || mainWindow.isDestroyed()
    || mainWindow.webContents.isDestroyed() || mainWindow.webContents.isLoading() || !projects) return;
  const queued = pendingOpenFiles.splice(0);
  for (const filePath of queued) {
    // The queue has already passed through the duplicate guard.
    const target = mainWindow.webContents;
    void openProjectForWindow({ projects }, target, filePath).then(result => {
      if (!target.isDestroyed()) target.send(IPC.projectOpenExternal, result);
    });
  }
}
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

  webContents.setWindowOpenHandler(({ url, frameName }) => {
    const opener = BrowserWindow.fromWebContents(webContents);
    if (isPanelPopoutRequest(url, frameName,
      !!opener && editors.has(opener) && isAllowedNavigation(webContents.getURL(), devRendererUrl)
    )) {
      return {
        action: 'allow',
        overrideBrowserWindowOptions: {
          ...backgroundWindowOptions(isBackgroundTest),
          minWidth: 280,
          minHeight: 240,
          webPreferences: {
            ...backgroundWindowOptions(isBackgroundTest).webPreferences,
            contextIsolation: true,
            sandbox: true,
            nodeIntegration: false
          }
        }
      };
    }
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

function isAppMainFrame(webContents: WebContents | null, requestingUrl: string | undefined): boolean {
  if (!webContents || webContents.isDestroyed()) return false;
  const url = requestingUrl ?? webContents.getURL();
  return isAllowedNavigation(url, devRendererUrl);
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

interface EditorWindowOptions {
  /** Plays the first-run entrance animation and defers showing the window. */
  entrance?: boolean;
  onEntranceReady?: () => void;
  /** The project this window opens. Null lets the renderer pick, the way a
   *  single-window launch did. */
  projectId?: string | null;
}

/* Windows after the first are offset so a new one never lands exactly on the
   window it was opened from. */
function cascadeBounds(): { x: number; y: number } | null {
  const from = currentEditor();
  if (!from || from.isDestroyed()) return null;
  const { x, y } = from.getBounds();
  const area = screen.getDisplayMatching(from.getBounds()).workArea;
  const next = { x: x + 26, y: y + 26 };
  // Walking off the display resets to its top-left instead of opening a window
  // whose titlebar the user cannot reach.
  if (next.x + 480 > area.x + area.width || next.y + 320 > area.y + area.height) {
    return { x: area.x + 40, y: area.y + 40 };
  }
  return next;
}

function createWindow(options: EditorWindowOptions = {}): BrowserWindow {
  const { entrance = false, onEntranceReady, projectId = null } = options;
  const testOptions = backgroundWindowOptions(isBackgroundTest);
  const cascade = editors.size > 0 ? cascadeBounds() : null;
  const window = new BrowserWindow({
    ...testOptions,
    ...(entrance ? { show: false } : {}),
    ...(cascade ?? {}),
    width: 1440,
    height: 900,
    minWidth: 980,
    minHeight: 640,
    titleBarStyle: 'hiddenInset',
    trafficLightPosition: { x: 14, y: 15 },
    // The window canvas is a native material: the desktop blurs through
    // behind the panels, and it goes flat when the window loses focus, the
    // way Finder and Xcode do. The renderer paints only a tint over it
    // (see `--bg-window` in css/app.css), so no opaque backgroundColor here.
    vibrancy: 'sidebar',
    visualEffectState: 'followWindow',
    webPreferences: {
      ...testOptions.webPreferences,
      preload: path.join(__dirname, '../preload/index.js'),
      contextIsolation: true,
      sandbox: true,
      nodeIntegration: false,
      autoplayPolicy: 'no-user-gesture-required'
    }
  });

  editors.add(window, projectId);
  persistOpenWindows();
  window.on('focus', () => editors.touch(window));
  window.webContents.once('did-finish-load', drainPendingOpenFiles);
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
    editors.remove(window);
    persistOpenWindows();
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

/** The project id in a window request, or null for "no project". */
function readProjectId(payload: unknown, channel: string): string | null {
  if (payload === null || payload === undefined) return null;
  if (typeof payload !== 'object') throw new Error(`${channel}: expected { projectId }`);
  const value = (payload as { projectId?: unknown }).projectId;
  if (value === null || value === undefined || value === '') return null;
  if (typeof value !== 'string' || !PROJECT_ID.test(value)) {
    throw new Error(`${channel}: invalid projectId`);
  }
  return value;
}

function registerWindowIpc(): void {
  const senderWindow = (event: { sender: WebContents }): BrowserWindow | null => {
    const window = BrowserWindow.fromWebContents(event.sender);
    return window && editors.has(window) ? window : null;
  };

  // Read during the renderer's synchronous boot, before it decides which
  // project to load, so it must answer without a round trip of its own.
  ipcMain.on(IPC.windowInitialProject, (event) => {
    const window = isTrustedSender(event) ? senderWindow(event) : null;
    if (!window) {
      // Nothing about the open documents is told to a sender we do not trust.
      event.returnValue = { projectId: null, taken: [] } satisfies WindowInitialProject;
      return;
    }
    const projectId = editors.projectOf(window);
    event.returnValue = {
      projectId,
      taken: editors.openProjectIds().filter((id) => id !== projectId)
    } satisfies WindowInitialProject;
  });

  // A window taking a document over as its own. Refused when another window
  // already has it, because two editors autosaving one slot would lose edits.
  ipcMain.handle(IPC.windowClaimProject, (event, payload: unknown): WindowClaimResult => {
    if (!isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    const window = senderWindow(event);
    if (!window) return { claimed: false, focused: false };
    const projectId = readProjectId(payload, IPC.windowClaimProject);
    const holder = projectId ? editors.windowForProject(projectId) : null;
    if (holder && holder !== window) {
      editors.reveal(holder);
      return { claimed: false, focused: true };
    }
    editors.claim(window, projectId);
    persistOpenWindows();
    return { claimed: true, focused: false };
  });

  ipcMain.handle(IPC.windowOpenProject, (event, payload: unknown): WindowOpenResult => {
    if (!isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    const projectId = readProjectId(payload, IPC.windowOpenProject);
    if (!projectId) return { opened: false, focused: false, error: 'A project is required' };
    const holder = editors.windowForProject(projectId);
    if (holder) {
      editors.reveal(holder);
      return { opened: false, focused: true };
    }
    createWindow({ projectId });
    return { opened: true, focused: false };
  });

  ipcMain.handle(IPC.windowNew, (event) => {
    if (!isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    createWindow();
  });

  // Routed through the window's own close, so the unsaved-changes guard runs.
  ipcMain.on(IPC.windowClose, (event) => {
    if (!isTrustedSender(event)) return;
    senderWindow(event)?.close();
  });
}

/** Reopens last session's windows, or opens one window when the preference is
 *  off, nothing was open, or those projects are gone. */
function restoreWindows(store: Store): void {
  const snapshot = store.get
    ? Object.fromEntries(['restoreWindows', 'projects', 'openWindows', 'openTabs'].map(key => [key, store.get!(key)]))
    : store.snapshot();
  if (snapshot['restoreWindows'] === false) {
    createWindow();
    return;
  }
  const metas = Array.isArray(snapshot['projects']) ? snapshot['projects'] : [];
  const known = new Set(
    metas.map((meta) => (meta as { id?: unknown } | null)?.id).filter((id): id is string => typeof id === 'string')
  );
  const ids = restorableProjects(snapshot['openWindows'], snapshot['openTabs'], (id) => known.has(id));
  if (!ids.length) {
    createWindow();
    return;
  }
  for (const projectId of ids) createWindow({ projectId });
}

if (!hasSingleInstanceLock) {
  app.quit();
} else {
  app.on('open-file', (event, filePath) => {
    event.preventDefault();
    queueOrOpen(filePath);
  });

  app.on('second-instance', (_event, argv) => {
    if (isBackgroundTest) return;
    const projectFiles = argv.filter(candidate => path.isAbsolute(candidate) && candidate.toLowerCase().endsWith('.pmv'));
    // The listener is installed before async startup finishes. Do not let an
    // early second launch create the editor before the first-run gate decides
    // whether onboarding owns startup.
    if (!startupInitialized) {
      for (const filePath of projectFiles) queueOrOpen(filePath);
      return;
    }
    if (onboardingFlow?.hasActiveWindow() || onboardingFlow?.isFirstRunPending()) {
      onboardingFlow.focus();
      for (const filePath of projectFiles) queueOrOpen(filePath);
      return;
    }
    // A second launch raises the window the user was last in rather than
    // adding one; New Window is how they ask for another.
    if (!editors.reveal(currentEditor())) createWindow();
    for (const filePath of projectFiles) queueOrOpen(filePath);
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
    const mediaProxies = new MediaProxyService(app.getPath('temp'), playbackConverter(proxyEncoder), imageSequenceConverter(proxyEncoder), previewConverter(proxyEncoder), stillImageConverter(proxyEncoder));
    registerAppProtocol();
    installPermissionHandlers(session.defaultSession, isAppMainFrame);

    // Boot barrier: the legacy renderer reads PM.store synchronously while its
    // scripts load, so the store must be in memory before the window exists.
    configureRuntimeUpdates({ root: path.join(app.getPath('userData'), 'runtimes'), appVersion: app.getVersion() });
    const store = createStore(path.join(app.getPath('userData'), 'store'));
    await store.load();
    registerStoreIpc(ipcMain, store, {
      isTrustedSender,
      // One window's write is another window's stale cache. Tell the others
      // which keys moved so their next read goes back to the store.
      broadcastChange: (keys, sender) => {
        for (const window of editors.all()) {
          if (window.webContents === sender || window.webContents.isDestroyed()) continue;
          window.webContents.send(IPC.storeChanged, { keys });
        }
      }
    });
    // Quitting closes every window in turn; the restore list must survive that,
    // so it stops tracking as soon as the quit begins.
    let quitting = false;
    app.on('before-quit', () => { quitting = true; });
    persistOpenWindows = () => {
      if (quitting) return;
      store.set('openWindows', editors.openProjectIds());
    };
    registerWindowIpc();
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
        .then(() => {
          extensionRegistry.emitChanged({ ids: [], reason: 'reload' });
          return startExtensionWatcher({ userDir, buildDir, registry: extensionRegistry });
        })
        .catch((error) => console.error('[extensions] initial refresh failed', error));
    } catch (error) {
      console.error('[extensions] boot skipped', error);
    }

    const ctx = { isTrustedSender, isTrustedSenderContents };
    projects = new ProjectFiles(path.join(app.getPath('userData'), 'project-files.json'), path.join(app.getPath('userData'), 'backups'));
    registerSaveIpc(ipcMain, { ...ctx, projects });
    registerCaptureIpc(ipcMain, ctx);
    registerShellIpc(ipcMain, { ...ctx, attachmentCacheDirectory: path.join(app.getPath('userData'), 'Attachment Cache') });
    registerThemeIpc(ipcMain, ctx);
    registerHapticsIpc(ipcMain, ctx);
    registerFontsIpc(ipcMain, ctx);
    registerContextMenuIpc(ipcMain, ctx);
    registerConfirmIpc(ipcMain, ctx);
    registerCloudMediaIpc(ipcMain, ctx);
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
      ['samples/gradient-tint/manifest.json', 'docs/samples/gradient-tint/manifest.json'],
      ['samples/gradient-tint/index.ts', 'docs/samples/gradient-tint/index.ts'],
      ['samples/gradient-tint/README.md', 'docs/samples/gradient-tint/README.md'],
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
      getWindow: () => currentEditor(),
      broadcast: (send) => {
        for (const window of editors.all()) {
          if (!window.webContents.isDestroyed()) send(window.webContents);
        }
      },
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
      agentToolCommandArgs: [...(app.isPackaged ? [] : [app.getAppPath()]), '--powermove-agent-tools'],
      refreshExtensions: refreshRestoredExtensions,
      openExternal: async (url) => { await shell.openExternal(url); }
    });
    onboardingFlow = new OnboardingFlow(ipcMain, {
      appOrigin: (devRendererUrl ?? APP_ORIGIN).replace(/\/$/, ''),
      displayBounds: () => {
        const editor = currentEditor();
        if (editor && !editor.isDestroyed()) {
          return screen.getDisplayMatching(editor.getBounds()).bounds;
        }
        return screen.getDisplayNearestPoint(screen.getCursorScreenPoint()).bounds;
      },
      backgroundTest: isBackgroundTest,
      userData: app.getPath('userData'),
      createEditor: () => {
        const existing = currentEditor();
        if (existing && !existing.isDestroyed()) {
          if (existing.isMinimized()) existing.restore();
          if (!isBackgroundTest) {
            existing.show();
            existing.focus();
          }
          return existing;
        }
        return new Promise<BrowserWindow>((resolve, reject) => {
          const editor = createWindow({ entrance: true, onEntranceReady: () => resolve(editor) });
          editor.webContents.once('did-fail-load', (_event, _code, description) => {
            editor.destroy();
            reject(new Error(description));
          });
        });
      },
      secure: (window) => secureWebContents(window.webContents, devRendererUrl)
    });
    const menu = installMenu(() => currentEditor(), {
      newWindow: () => createWindow(),
      closeWindow: () => currentEditor()?.close()
    });
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
      restoreWindows(store);
    }
    startupInitialized = true;
    drainPendingOpenFiles();

    app.on('activate', () => {
      if (isBackgroundTest) return;
      if (onboardingFlow?.hasActiveWindow() || onboardingFlow?.isFirstRunPending()) {
        onboardingFlow.focus();
        return;
      }
      if (editors.size === 0) createWindow();
      else editors.reveal(currentEditor());
    });
  });
}

app.on('window-all-closed', () => {
  if (process.platform !== 'darwin') {
    app.quit();
  }
});
