/*
 * `powermove serve`: the Powermove host without Electron. The same main-process
 * modules that back the desktop app run here in plain Node, and each browser
 * tab that connects over WebSocket plays the part of one editor window. The
 * renderer bundle is served as static files; the browser-side bridge in
 * src/renderer/src/host/web-bridge.ts replaces the preload.
 *
 * Agents (Codex, Claude) run on this machine, so a laptop can close while a
 * Linux box keeps working on the project.
 */
import { createServer as createHttpServer, type IncomingMessage, type ServerResponse } from 'node:http';
import { createServer as createHttpsServer } from 'node:https';
import { createReadStream } from 'node:fs';
import { mkdir, open, readFile, readdir, realpath, rm, stat, writeFile, type FileHandle } from 'node:fs/promises';
import path from 'node:path';
import { randomBytes, randomUUID, timingSafeEqual } from 'node:crypto';
import type { Duplex } from 'node:stream';

import { WebSocketServer, type WebSocket } from 'ws';

import { IPC, PROJECT_ID, type WindowClaimResult, type WindowInitialProject, type WindowOpenResult } from '../shared/ipc';
import { WEB, WEB_UPLOAD_CHUNK_BYTES, type WebHello } from '../shared/wire';
import { isRecord } from '../shared/guards';
import { CONTENT_SECURITY_POLICY } from '../main/security-policy';
import { MIME_TYPES } from '../main/mime';
import { createStore, registerStoreIpc } from '../main/storage';
import { openProjectForWindow, registerSaveIpc } from '../main/save';
import { ProjectFiles } from '../main/project-files';
import { registerLogIpc } from '../main/log';
import { registerShellIpc } from '../main/shell';
import { registerCaptureIpc } from '../main/capture';
import { registerRenderEncoder } from '../main/render-encoder';
import { MediaProxyService, imageSequenceConverter, playbackConverter, previewConverter, registerMediaProxyIpc, stillImageConverter } from '../main/media-proxy';
import { registerCodexIpc } from '../main/codex';
import { recoverAllInterruptedExtensionTransactions } from '../main/codex/change-history';
import { registerExtensionsIpc, serveExtensionAsset } from '../main/extensions';
import { createExtensionRegistry } from '../main/extensions/registry';
import { startExtensionWatcher } from '../main/extensions/watcher';
import { EditorWindows } from '../main/windows';
import { app, configureElectronStub, type MessageBoxOptions } from './electron-stub';
import { RemoteClient, RemoteWindow, WebIpcMain, type RemoteEvent } from './clients';
import { reachableAddresses } from './addresses';
import { ensureCertificate } from './tls';
import { ProjectSessions } from './sessions';
import type { BrowserWindow, IpcMain, WebContents } from 'electron';

/** Electron's own types for main's modules; the remote stand-ins have the members they use. */
const asWebContents = (client: RemoteClient): WebContents => client as unknown as WebContents;
const asBrowserWindow = (window: RemoteWindow | null): BrowserWindow | null => window as unknown as BrowserWindow | null;

export interface ServeOptions {
  host: string;
  port: number;
  /** Profile directory: store, extensions, agent workspaces. */
  userData: string;
  /** The Vite-built renderer (out/renderer). */
  rendererDir: string;
  /** builtin-extensions/, api-pack/, agent-tools/ live here. */
  resourcesDir: string;
  /** Directory whose node_modules holds ffmpeg-static (render-encoder resolves it from here). */
  appPath: string;
  /** Where "Save…" writes on this machine. */
  exportsDir: string;
  version: string;
  codexBinary: string | null;
  claudeBinary: string | null;
  /** Bearer token. Generated and persisted under userData when omitted. */
  token?: string;
  /** Plain http instead of a self-signed https. Only for a TLS-terminating proxy in front. */
  insecure?: boolean;
  log?: (line: string) => void;
}

export interface RunningServer {
  url: string;
  urls: string[];
  token: string;
  close(): Promise<void>;
}

const COOKIE = 'pm_session';
const WS_PATH = '/__powermove/ws';
const SANDBOX_PATH = 'host/sandbox.html';
/** The sandbox document's own policy: eval inside, no network, same-origin script only. */
const SANDBOX_CSP = "default-src 'none'; script-src 'self' 'unsafe-eval'; worker-src blob:; connect-src 'none'";
const MAX_UPLOAD_BYTES = 4 * 1024 * 1024 * 1024;
const API_PACK: Array<[name: string, devPath: string]> = [
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

/* ── auth ─────────────────────────────────────────────────── */

async function loadOrCreateToken(userData: string, provided?: string): Promise<string> {
  if (provided) return provided;
  const file = path.join(userData, 'serve-token');
  try {
    const existing = (await readFile(file, 'utf8')).trim();
    if (/^[a-f0-9]{48}$/.test(existing)) return existing;
  } catch { /* first run */ }
  const token = randomBytes(24).toString('hex');
  await mkdir(userData, { recursive: true });
  await writeFile(file, token, { mode: 0o600 });
  return token;
}

function tokenMatches(expected: string, candidate: string | undefined | null): boolean {
  if (!candidate || candidate.length !== expected.length) return false;
  return timingSafeEqual(Buffer.from(expected), Buffer.from(candidate));
}

function cookieValue(request: IncomingMessage, name: string): string | null {
  const header = request.headers.cookie;
  if (!header) return null;
  for (const part of header.split(';')) {
    const [key, ...rest] = part.trim().split('=');
    if (key === name) return decodeURIComponent(rest.join('='));
  }
  return null;
}

function isLoopback(host: string): boolean {
  return host === '127.0.0.1' || host === 'localhost' || host === '::1';
}

/* ── static files ────────────────────────────────────────── */

function headers(contentType: string, extra: Record<string, string> = {}): Record<string, string> {
  return { 'Content-Security-Policy': CONTENT_SECURITY_POLICY, 'Content-Type': contentType, 'X-Content-Type-Options': 'nosniff', ...extra };
}

function text(response: ServerResponse, status: number, body: string): void {
  response.writeHead(status, headers('text/plain; charset=utf-8'));
  response.end(body);
}

async function serveStatic(rendererRoot: string, requestedPath: string, response: ServerResponse): Promise<void> {
  const filePath = path.resolve(rendererRoot, requestedPath === '' ? 'index.html' : requestedPath);
  const relative = path.relative(rendererRoot, filePath);
  if (relative === '..' || relative.startsWith(`..${path.sep}`) || path.isAbsolute(relative)) { text(response, 403, 'Forbidden'); return; }
  let info;
  try { info = await stat(filePath); } catch { text(response, 404, 'Not found'); return; }
  if (!info.isFile()) { text(response, 404, 'Not found'); return; }
  const contentType = MIME_TYPES[path.extname(filePath).toLowerCase()] ?? 'application/octet-stream';
  const head = headers(contentType, { 'Content-Length': String(info.size), 'Cache-Control': relative.startsWith('assets/') ? 'public, max-age=31536000, immutable' : 'no-cache' });
  if (relative === SANDBOX_PATH) head['Content-Security-Policy'] = SANDBOX_CSP;
  response.writeHead(200, head);
  createReadStream(filePath).pipe(response);
}

/* ── uploads from the browser (drag-drop media, Open Project…) ── */

interface Upload { id: string; handle: FileHandle; filePath: string; size: number; received: number; owner: RemoteClient }

function safeFileName(value: unknown): string | null {
  if (typeof value !== 'string' || value.length === 0 || value.length > 255 || value.includes('\0')) return null;
  const name = path.basename(value.replace(/[\\/]/g, '_'));
  return name === '.' || name === '..' || name === '' ? null : name;
}

/* ── the host ─────────────────────────────────────────────── */

export async function serve(options: ServeOptions): Promise<RunningServer> {
  const log = options.log ?? ((line: string) => console.log(line));
  const userData = path.resolve(options.userData);
  await mkdir(userData, { recursive: true });
  const token = await loadOrCreateToken(userData, options.token);
  const ipc = new WebIpcMain();
  // Main's modules type their argument as Electron's IpcMain but only use
  // handle/on/removeListener, which WebIpcMain provides with the same shapes.
  const ipcMain = ipc as unknown as IpcMain;
  const editors = new EditorWindows<RemoteWindow>();

  const askMessageBox = async (window: RemoteWindow | null, box: MessageBoxOptions): Promise<number> => {
    const client = window?.webContents ?? ipc.current();
    const cancel = box.cancelId ?? Math.max(0, (box.buttons?.length ?? 1) - 1);
    if (!client) return cancel;
    try {
      const answer = await client.ask<number>(WEB.askMessageBox, { message: box.message, detail: box.detail ?? null, buttons: box.buttons ?? ['OK'], defaultId: box.defaultId ?? 0, cancelId: cancel, type: box.type ?? 'question' });
      return typeof answer === 'number' && Number.isInteger(answer) && answer >= 0 && answer < (box.buttons?.length ?? 1) ? answer : cancel;
    } catch { return cancel; }
  };
  configureElectronStub({
    ipc, userData, appPath: options.appPath, resourcesPath: options.resourcesDir, version: options.version, exportsDir: options.exportsDir,
    askMessageBox,
    openExternal: (url) => { for (const client of ipc.all()) client.send(WEB.openExternal, url); log(`[serve] open in your browser: ${url}`); }
  });
  app.isPackaged = false;

  // Every socket was authenticated at upgrade, so a live client is a trusted
  // sender. Typed loosely because main's modules pass Electron's event types.
  const isTrustedSender = (event: { sender: unknown; senderFrame?: unknown }): boolean =>
    event.sender instanceof RemoteClient && !event.sender.isDestroyed() && event.senderFrame !== null;
  const isTrustedSenderContents = (sender: unknown): boolean => sender instanceof RemoteClient && !sender.isDestroyed();
  const ctx = { isTrustedSender, isTrustedSenderContents };
  const trustedClient = (event: RemoteEvent): RemoteClient => {
    if (!isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    return event.sender;
  };

  /* store */
  const store = createStore(path.join(userData, 'store'));
  await store.load();
  registerStoreIpc(ipcMain, store, {
    isTrustedSender,
    broadcastChange: (keys, sender) => { for (const client of ipc.all()) if (client !== sender) client.send(IPC.storeChanged, { keys }); }
  });

  /* project sessions: every tab on a project shares the host's document */
  const sessions = new ProjectSessions({
    read: (key) => { const raw = store.getSerialized?.(key); return raw == null ? store.snapshot()[key] ?? null : JSON.parse(raw); },
    set: (key, value) => store.set(key, value)
  }, { channel: WEB.syncPatch, version: options.version, log });
  ipc.handle(WEB.syncJoin, (event, payload) => {
    const client = trustedClient(event);
    if (!isRecord(payload) || typeof payload.projectId !== 'string' || !PROJECT_ID.test(payload.projectId)) throw new Error('sync: expected { projectId, doc }');
    return sessions.join(client, payload.projectId, payload.doc ?? null);
  });
  ipc.on(WEB.syncLeave, (event, projectId) => {
    if (isTrustedSender(event) && typeof projectId === 'string') sessions.leave(event.sender, projectId);
  });
  ipc.handle(WEB.syncPatch, (event, payload) => {
    const client = trustedClient(event);
    if (!isRecord(payload) || typeof payload.projectId !== 'string') throw new Error('sync: expected { projectId, patches }');
    return sessions.patch(client, payload.projectId, payload.patches);
  });

  /* extensions */
  const userDir = path.join(userData, 'extensions');
  const buildDir = path.join(userData, 'extensions-build');
  const builtinResourcesDir = path.join(options.resourcesDir, 'builtin-extensions');
  let refreshRestoredExtensions: ((ids: string[]) => Promise<void>) | undefined;
  let stopWatcher: (() => void) | undefined;
  try {
    await recoverAllInterruptedExtensionTransactions(userData, userDir);
    await mkdir(userDir, { recursive: true });
    let builtinIds: string[] = [];
    try {
      builtinIds = (await readdir(builtinResourcesDir, { withFileTypes: true })).filter((entry) => entry.isDirectory() && !entry.name.startsWith('.')).map((entry) => entry.name).sort();
    } catch { /* partial checkout: kernel still boots */ }
    const registry = createExtensionRegistry({ store, userDir, buildDir, builtinIds, resourcesDir: builtinResourcesDir });
    registerExtensionsIpc(ipcMain, { registry, resourcesDir: builtinResourcesDir, isTrusted: isTrustedSender });
    refreshRestoredExtensions = async (ids) => { await registry.refresh(ids); registry.emitChanged({ ids, reason: 'reload' }); };
    void registry.refresh().then(() => {
      registry.emitChanged({ ids: [], reason: 'reload' });
      const watcher = startExtensionWatcher({ userDir, buildDir, registry });
      stopWatcher = () => { void watcher.close(); };
    }).catch((error) => log(`[extensions] initial refresh failed: ${String(error)}`));
  } catch (error) {
    log(`[extensions] boot skipped: ${String(error)}`);
  }

  /* files, media, render */
  const projects = new ProjectFiles(path.join(userData, 'project-files.json'), path.join(userData, 'backups'));
  registerSaveIpc(ipcMain, { ...ctx, projects });
  registerCaptureIpc(ipcMain, ctx);
  registerShellIpc(ipcMain, { ...ctx, attachmentCacheDirectory: path.join(userData, 'Attachment Cache') });
  registerLogIpc(ipcMain, ctx);
  const ffmpeg = path.join(options.appPath, 'node_modules', 'ffmpeg-static', 'ffmpeg');
  const mediaProxies = new MediaProxyService(app.getPath('temp'), playbackConverter(ffmpeg), imageSequenceConverter(ffmpeg), previewConverter(ffmpeg), stillImageConverter(ffmpeg));
  registerMediaProxyIpc(ipcMain, mediaProxies, ctx);
  registerRenderEncoder(ipcMain, ctx);

  /* agents */
  const apiPackDir = path.join(options.resourcesDir, 'api-pack');
  registerCodexIpc(ipcMain, {
    getWindow: () => asBrowserWindow(ipc.current()?.window ?? null),
    broadcast: (send) => { for (const client of ipc.all()) send(asWebContents(client)); },
    userData,
    extensionsDir: userDir,
    apiPackFiles: async () => {
      const files: Array<{ name: string; text: string }> = [];
      for (const [name] of API_PACK) {
        try { files.push({ name, text: await readFile(path.join(apiPackDir, name), 'utf8') }); } catch (error) { log(`[api-pack] missing ${name}: ${String(error)}`); }
      }
      return files;
    },
    isTrustedSender,
    codexBinaryPref: () => options.codexBinary,
    claudeBinaryPref: () => options.claudeBinary,
    agentToolServerPath: path.join(options.resourcesDir, 'agent-tools', 'mcp-server.mjs'),
    agentToolCommand: process.execPath,
    refreshExtensions: refreshRestoredExtensions,
    builtinExtensionsDir: builtinResourcesDir,
    openExternal: async (url) => { for (const client of ipc.all()) client.send(WEB.openExternal, url); log(`[serve] sign in from your browser: ${url}`); }
  });

  /* windows: one per browser tab */
  const readProjectId = (payload: unknown, channel: string): string | null => {
    if (payload === null || payload === undefined) return null;
    if (typeof payload !== 'object') throw new Error(`${channel}: expected { projectId }`);
    const value = (payload as { projectId?: unknown }).projectId;
    if (value === null || value === undefined || value === '') return null;
    if (typeof value !== 'string' || !PROJECT_ID.test(value)) throw new Error(`${channel}: invalid projectId`);
    return value;
  };
  ipc.on(IPC.windowInitialProject, (event) => {
    const window = isTrustedSender(event) ? event.sender.window : null;
    if (!window || !editors.has(window)) { event.returnValue = { projectId: null, taken: [] } satisfies WindowInitialProject; return; }
    // Tabs share documents, so nothing is "taken"; a tab opened without a
    // project mirrors the one the user was last in.
    const own = editors.projectOf(window);
    const recent = editors.all().filter((other) => other !== window).map((other) => editors.projectOf(other)).find((id) => !!id) ?? null;
    const projectId = own ?? recent;
    if (projectId && !own) editors.claim(window, projectId);
    event.returnValue = { projectId, taken: [] } satisfies WindowInitialProject;
  });
  ipc.handle(IPC.windowClaimProject, (event, payload): WindowClaimResult => {
    const window = trustedClient(event).window;
    const projectId = readProjectId(payload, IPC.windowClaimProject);
    // Unlike desktop windows, tabs share a project through its session, so a
    // claim never bounces to another tab.
    editors.claim(window, projectId);
    return { claimed: true, focused: false };
  });
  ipc.handle(IPC.windowOpenProject, (event, payload): WindowOpenResult => {
    if (!isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    const projectId = readProjectId(payload, IPC.windowOpenProject);
    if (!projectId) return { opened: false, focused: false, error: 'A project is required' };
    event.sender.send(WEB.openWindow, projectId);
    return { opened: true, focused: false };
  });
  ipc.handle(IPC.windowNew, (event) => { if (!isTrustedSender(event)) throw new Error('Unauthorized IPC sender'); event.sender.send(WEB.openWindow, null); });
  ipc.on(IPC.windowClose, (event) => { if (isTrustedSender(event)) event.sender.send(WEB.closeWindow); });
  ipc.handle(IPC.ping, (event) => { if (!isTrustedSender(event)) throw new Error('Unauthorized IPC sender'); return 'pong'; });

  /* web-only channels */
  ipc.handle(WEB.hello, (event): WebHello => {
    if (!isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    return { node: process.versions.node, version: options.version, platform: process.platform, exportsDir: options.exportsDir };
  });
  const uploads = new Map<string, Upload>();
  const uploadsRoot = { media: path.join(userData, 'Remote Uploads'), project: path.join(userData, 'Remote Projects') };
  const discardUpload = async (upload: Upload, keepFile: boolean): Promise<void> => {
    uploads.delete(upload.id);
    await upload.handle.close().catch(() => undefined);
    if (!keepFile) await rm(path.dirname(upload.filePath), { recursive: true, force: true });
  };
  ipc.handle(WEB.uploadBegin, async (event, payload) => {
    if (!isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    if (!isRecord(payload)) throw new Error('upload: expected { name, size, kind }');
    const name = safeFileName(payload.name);
    const size = payload.size;
    const kind = payload.kind === 'project' ? 'project' : 'media';
    if (!name || typeof size !== 'number' || !Number.isSafeInteger(size) || size < 0 || size > MAX_UPLOAD_BYTES) throw new Error('upload: invalid name or size');
    const id = randomUUID();
    const dir = path.join(uploadsRoot[kind], id);
    await mkdir(dir, { recursive: true });
    const filePath = path.join(dir, name);
    const handle = await open(filePath, 'wx', 0o600);
    const upload: Upload = { id, handle, filePath, size, received: 0, owner: event.sender };
    uploads.set(id, upload);
    log(`[serve] client ${event.sender.id}: uploading ${name} (${(size / 1048576).toFixed(1)} MB)`);
    event.sender.once('destroyed', () => { if (uploads.get(id) === upload) void discardUpload(upload, false); });
    return id;
  });
  ipc.handle(WEB.uploadChunk, async (event, payload) => {
    if (!isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    const { id, offset, data } = (payload ?? {}) as { id?: unknown; offset?: unknown; data?: unknown };
    const upload = typeof id === 'string' ? uploads.get(id) : undefined;
    if (!upload || upload.owner !== event.sender) throw new Error('upload: unknown upload');
    // Chunks arrive a few at a time and out of order; each names its offset.
    if (!(data instanceof Uint8Array) || data.byteLength === 0 || data.byteLength > WEB_UPLOAD_CHUNK_BYTES
      || typeof offset !== 'number' || !Number.isSafeInteger(offset) || offset < 0 || offset % WEB_UPLOAD_CHUNK_BYTES !== 0
      || offset + data.byteLength > upload.size) throw new Error('upload: invalid chunk');
    await upload.handle.write(data, 0, data.byteLength, offset);
    upload.received += data.byteLength;
  });
  ipc.handle(WEB.uploadFinish, async (event, id) => {
    if (!isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    const upload = typeof id === 'string' ? uploads.get(id) : undefined;
    if (!upload || upload.owner !== event.sender) throw new Error('upload: unknown upload');
    if (upload.received !== upload.size) { await discardUpload(upload, false); throw new Error('upload: incomplete'); }
    await discardUpload(upload, true);
    log(`[serve] client ${event.sender.id}: upload complete ${path.basename(upload.filePath)}`);
    return upload.filePath;
  });
  ipc.handle(WEB.uploadAbort, async (event, id) => {
    const upload = typeof id === 'string' ? uploads.get(id) : undefined;
    if (upload && upload.owner === event.sender) await discardUpload(upload, false);
  });
  ipc.handle(WEB.projectOpenPath, async (event, filePath) => {
    if (!isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    if (typeof filePath !== 'string') throw new Error('open: expected a path');
    const resolved = await realpath(filePath);
    const root = await realpath(uploadsRoot.project).catch(() => uploadsRoot.project);
    const relative = path.relative(root, resolved);
    if (!relative || relative.startsWith('..') || path.isAbsolute(relative)) throw new Error('open: path is not an uploaded project');
    const result = await openProjectForWindow({ projects }, asWebContents(event.sender), resolved);
    log(`[serve] client ${event.sender.id}: open ${path.basename(resolved)} → ${result.ok ? 'ok' : `failed: ${result.error ?? 'cancelled'}`}`);
    return result;
  });

  /* http + ws */
  const rendererRoot = path.resolve(options.rendererDir);
  const exportsRoot = path.resolve(options.exportsDir);
  const authorized = (request: IncomingMessage): boolean => tokenMatches(token, cookieValue(request, COOKIE));
  const handler = async (request: IncomingMessage, response: ServerResponse): Promise<void> => {
    try {
      const url = new URL(request.url ?? '/', 'http://localhost');
      const presented = url.searchParams.get('token');
      if (presented !== null) {
        if (!tokenMatches(token, presented)) { text(response, 403, 'Wrong token. Copy the URL printed by `powermove serve`.'); return; }
        url.searchParams.delete('token');
        response.writeHead(302, {
          'Set-Cookie': `${COOKIE}=${token}; Path=/; HttpOnly; SameSite=Strict; Max-Age=31536000${options.insecure ? '' : '; Secure'}`,
          Location: `${url.pathname}${url.search}`
        });
        response.end();
        return;
      }
      if (!authorized(request)) { text(response, 401, 'Powermove serve: open the URL with the token printed in the terminal.'); return; }
      if (request.method !== 'GET' && request.method !== 'HEAD') { text(response, 405, 'Method not allowed'); return; }
      const pathname = decodeURIComponent(url.pathname);
      if (pathname.startsWith('/ext/')) {
        const asset = await serveExtensionAsset(pathname);
        if (!asset) { text(response, 404, 'Not found'); return; }
        const body = Buffer.from(await asset.arrayBuffer());
        response.writeHead(asset.status, headers('text/javascript; charset=utf-8', { 'Cache-Control': 'no-store', 'Content-Length': String(body.byteLength) }));
        response.end(body);
        return;
      }
      if (pathname === '/__powermove/download') {
        const requested = url.searchParams.get('path') ?? '';
        const resolved = await realpath(requested).catch(() => null);
        const root = await realpath(exportsRoot).catch(() => exportsRoot);
        const relative = resolved ? path.relative(root, resolved) : '..';
        if (!resolved || !relative || relative.startsWith('..') || path.isAbsolute(relative)) { text(response, 404, 'Not found'); return; }
        const info = await stat(resolved);
        response.writeHead(200, headers(MIME_TYPES[path.extname(resolved).toLowerCase()] ?? 'application/octet-stream', {
          'Content-Length': String(info.size),
          'Content-Disposition': `attachment; filename*=UTF-8''${encodeURIComponent(path.basename(resolved))}`
        }));
        createReadStream(resolved).pipe(response);
        return;
      }
      await serveStatic(rendererRoot, pathname.replace(/^\/+/, ''), response);
    } catch (error) {
      log(`[serve] ${request.method} ${request.url}: ${error instanceof Error ? error.message : String(error)}`);
      if (!response.headersSent) text(response, 500, 'Internal error');
      else response.end();
    }
  };
  const scheme = options.insecure ? 'http' : 'https';
  const server = options.insecure
    ? createHttpServer(handler)
    : createHttpsServer(await ensureCertificate(userData), handler);
  const sockets = new WebSocketServer({ noServer: true, maxPayload: 64 * 1024 * 1024 });
  server.on('upgrade', (request: IncomingMessage, socket: Duplex, head: Buffer) => {
    const url = new URL(request.url ?? '/', 'http://localhost');
    if (url.pathname !== WS_PATH || !authorized(request)) { socket.write('HTTP/1.1 401 Unauthorized\r\n\r\n'); socket.destroy(); return; }
    // The browser tab was opened for a project (window.open from another tab).
    const requested = url.searchParams.get('project');
    const projectId = requested && PROJECT_ID.test(requested) ? requested : null;
    sockets.handleUpgrade(request, socket, head, (ws: WebSocket) => {
      const origin = `${request.headers.origin ?? `${scheme}://localhost`}/`;
      const client = ipc.connect({ send: (data) => ws.send(data), close: (code, reason) => ws.close(code, reason) }, origin);
      editors.add(client.window, projectId);
      log(`[serve] client ${client.id} connected${projectId ? ` (project ${projectId})` : ''}`);
      ws.on('message', (data: Buffer | ArrayBuffer | Buffer[], isBinary: boolean) => {
        if (!isBinary) return;
        const bytes = Array.isArray(data) ? Buffer.concat(data) : data instanceof ArrayBuffer ? new Uint8Array(data) : new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
        client.receive(bytes);
      });
      ws.on('close', () => { log(`[serve] client ${client.id} disconnected`); editors.remove(client.window); client.destroy(); });
      ws.on('error', (error: Error) => log(`[serve] client ${client.id}: ${error.message}`));
    });
  });

  await new Promise<void>((resolve, reject) => {
    server.once('error', reject);
    server.listen(options.port, options.host, () => { server.off('error', reject); resolve(); });
  });
  const address = server.address();
  const port = typeof address === 'object' && address ? address.port : options.port;
  const urls = reachableAddresses(options.host, port, scheme).map((base) => `${base}/?token=${token}`);
  if (isLoopback(options.host)) log('[serve] bound to loopback only; pass --host 0.0.0.0 to reach it from another machine');

  return {
    url: urls[0]!,
    urls,
    token,
    async close() {
      stopWatcher?.();
      for (const client of ipc.all()) client.destroy();
      for (const socket of sockets.clients) socket.terminate();
      await new Promise<void>((resolve) => sockets.close(() => resolve()));
      // Keep-alive HTTP connections would otherwise hold close() open.
      server.closeAllConnections();
      await new Promise<void>((resolve) => server.close(() => resolve()));
      app.emit('will-quit');
      await mediaProxies.dispose();
      await sessions.flush();
      await store.flushAll();
    }
  };
}
