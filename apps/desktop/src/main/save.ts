import { agentExportDestination } from './agent-export';
import path from 'node:path';
import { randomUUID } from 'node:crypto';

import {
  BrowserWindow,
  dialog,
  type FileFilter,
  type IpcMain,
  type IpcMainInvokeEvent,
  type WebContents,
  type SaveDialogOptions,
  type SaveDialogReturnValue
} from 'electron';

import { isBytes, isRecord, isString, IpcValidationError } from '../shared/guards';
import { IPC, LIMITS, PROJECT_ID, type FileSaveResult, type ProjectOpenResult, type CloseDecision, type ProjectSaveBeginResult } from '../shared/ipc';
import { atomicWrite, type ProjectFiles } from './project-files';
import { rememberExport } from './export-sidecar';
import { FileUpload, FILE_CHUNK_BYTES } from './file-upload';
import { validateSaveMedia, type ProjectSave } from './project-save';
import { isRendererUnavailable, watchRendererDocument } from './renderer-recovery';

const MAX_SAVE_NAME_CHARS = 200;

const FILTER_NAMES: Readonly<Record<string, string>> = {
  gif: 'GIF Image',
  jpeg: 'JPEG Image',
  jpg: 'JPEG Image',
  json: 'JSON',
  mp3: 'MP3 Audio',
  mp4: 'MPEG-4 Video',
  pmv: 'Powermove Project',
  png: 'PNG Image',
  svg: 'SVG Image',
  wav: 'WAV Audio',
  webm: 'WebM Video'
};

export interface SaveDialogAdapter {
  showSave(window: BrowserWindow, options: SaveDialogOptions): Promise<SaveDialogReturnValue>;
}

export interface SaveIpcContext {
  isTrustedSender(event: IpcMainInvokeEvent): boolean;
  dialogs?: SaveDialogAdapter;
  projects?: ProjectFiles;
  recoveryProjects?: ProjectFiles;
}

type ReaderState = {
  readers: Map<string, { owner: WebContents; timer: ReturnType<typeof setTimeout> }>;
  watchedOwners: WeakSet<WebContents>;
};

const readerStates = new WeakMap<ProjectFiles, ReaderState>();

function projectReaderState(projects: ProjectFiles): ReaderState {
  let state = readerStates.get(projects);
  if (!state) {
    state = { readers: new Map(), watchedOwners: new WeakSet() };
    readerStates.set(projects, state);
  }
  return state;
}

async function closeProjectReader(projects: ProjectFiles, token: string, verify = false): Promise<void> {
  const state = projectReaderState(projects);
  const reader = state.readers.get(token);
  if (!reader) return;
  clearTimeout(reader.timer);
  state.readers.delete(token);
  await projects.close(token, verify);
}

function watchProjectReaderOwner(projects: ProjectFiles, owner: WebContents): void {
  const state = projectReaderState(projects);
  if (state.watchedOwners.has(owner)) return;
  state.watchedOwners.add(owner);
  const release = () => {
    state.watchedOwners.delete(owner);
    for (const [token, reader] of state.readers) {
      if (reader.owner === owner) void closeProjectReader(projects, token).catch(() => undefined);
    }
  };
  watchRendererDocument(owner, release);
}

export async function openProjectForWindow(
  ctx: { projects: ProjectFiles },
  webContents: WebContents,
  filePath: string, copy = false
): Promise<ProjectOpenResult> {
  // Recovery reuses WebContents. An open started by the old renderer must not
  // grant its completed reader to the replacement process.
  let rendererGone = false;
  const gone = () => { rendererGone = true; };
  const stopWatching = watchRendererDocument(webContents, gone);
  try {
    if (isRendererUnavailable(webContents)) throw new Error('Project window is unavailable');
    const opened = await ctx.projects.open(filePath, { allowDocumentRange: true, copy });
    if (rendererGone || isRendererUnavailable(webContents)) {
      await ctx.projects.close(opened.token, false).catch(() => undefined);
      throw new Error('Project window is unavailable');
    }
    const state = projectReaderState(ctx.projects);
    const timer = setTimeout(() => {
      void closeProjectReader(ctx.projects, opened.token).catch(() => undefined);
    }, 120_000);
    timer.unref();
    state.readers.set(opened.token, { owner: webContents, timer });
    watchProjectReaderOwner(ctx.projects, webContents);
    // Electron's context bridge deeply copies objects. Large PMV4 documents
    // use the existing bounded reader instead of copying their full history.
    if (opened.documentRange && opened.documentRange.length >= FILE_CHUNK_BYTES) {
      return { ok: true, ...(copy ? { copy: true } : {}), path: opened.path, projectId: opened.projectId, token: opened.token,
        size: opened.size, media: opened.media, documentRange: opened.documentRange };
    }
    return { ok: true, ...opened, ...(copy ? { copy: true } : {}) };
  } catch (error: any) {
    return { ok: false, cancelled: false, error: error.message || 'Could not open project.' };
  } finally { stopWatching(); }
}

/** Convert an untrusted suggestion into one plain filename. */
export function sanitizeSaveName(value: unknown): string | null {
  if (!isString(value)) return null;

  const withoutNul = value.replaceAll('\0', '');
  const components = withoutNul.split(/[\\/]+/).filter(Boolean);
  const name = components.at(-1) ?? '';

  if (
    name === '' ||
    name === '.' ||
    name === '..' ||
    name.length > MAX_SAVE_NAME_CHARS
  ) {
    return null;
  }
  return name;
}

export function saveFiltersForName(name: string): FileFilter[] | undefined {
  const extension = path.extname(name).slice(1).toLowerCase();
  if (!/^[a-z0-9]{1,20}$/.test(extension)) return undefined;

  return [
    {
      name: FILTER_NAMES[extension] ?? `${extension.toUpperCase()} File`,
      extensions: [extension]
    }
  ];
}

/** Desktop only: opens a project file the user picked in a renderer file
 *  input. The browser host never registers it, so remote clients cannot name
 *  arbitrary paths on the host. */
export function registerProjectOpenPathIpc(ipcMain: Pick<IpcMain, 'handle'>, ctx: Pick<SaveIpcContext, 'isTrustedSender' | 'projects'>): void {
  ipcMain.handle(IPC.projectOpenPath, async (event, filePath: unknown): Promise<ProjectOpenResult> => {
    if (!ctx.isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    if (!isString(filePath) || !path.isAbsolute(filePath) || !/\.(pmv|json)$/i.test(filePath)) {
      throw new IpcValidationError(IPC.projectOpenPath, 'invalid project path');
    }
    if (!ctx.projects) throw new Error('Project window is unavailable');
    return openProjectForWindow({ projects: ctx.projects }, event.sender, filePath);
  });
}

export function registerSaveIpc(ipcMain: Pick<IpcMain, 'handle'>, ctx: SaveIpcContext): void {
  const pending = new Map<object, symbol>();
  const claimPending = (owner: object) => { const marker = Symbol(); pending.set(owner, marker); return marker; };
  const releasePending = (owner: object, marker: symbol) => { if (pending.get(owner) === marker) pending.delete(owner); };
  const destinations = new Map<object, { token: string; path: string; directory: boolean }>();
  const watchedOwners = new WeakSet<object>();
  const uploads = new Map<object, { id: string; ready: Promise<FileUpload>; timer: ReturnType<typeof setTimeout>; disposing?: Promise<void> }>();
  const projectSaves = new Map<object, { id: string; ready: Promise<ProjectSave>; timer: ReturnType<typeof setTimeout>; finishing: boolean; marker: symbol; recovery?: boolean; disposing?: Promise<void> }>();
  const discardProjectSave = async (owner: object, id?: string) => {
    const save = projectSaves.get(owner);
    if (!save || (id && save.id !== id)) return;
    if (save.disposing) return save.disposing;
    clearTimeout(save.timer);
    save.disposing = (async () => {
      try { await (await save.ready.catch(() => undefined))?.dispose(); }
      finally {
        if (projectSaves.get(owner) === save) projectSaves.delete(owner);
        releasePending(owner, save.marker);
      }
    })();
    return save.disposing;
  };
  const discardUpload = async (owner: object, id?: string) => {
    const upload = uploads.get(owner);
    if (!upload || (id && id !== upload.id)) return;
    if (upload.disposing) return upload.disposing;
    clearTimeout(upload.timer);
    upload.disposing = (async () => {
      try { await (await upload.ready.catch(() => undefined))?.dispose(); }
      finally { if (uploads.get(owner) === upload) uploads.delete(owner); }
    })();
    return upload.disposing;
  };
  const watchOwner = (owner: IpcMainInvokeEvent['sender']) => {
    if (watchedOwners.has(owner)) return;
    watchedOwners.add(owner);
    const release = () => {
      watchedOwners.delete(owner);
      destinations.delete(owner);
      // A final transaction may already be publishing. Let it finish while
      // disposal waits; an abandoned native dialog can release its lock now.
      if (!projectSaves.get(owner)?.finishing) pending.delete(owner);
      void discardUpload(owner).catch(() => undefined);
      void discardProjectSave(owner).catch(() => undefined);
    };
    watchRendererDocument(owner, release);
  };
  const versionGrants = new WeakMap<object, Map<string, { projectId: string; path: string }>>();
  ipcMain.handle(IPC.projectVersions, async (event, projectId: unknown) => {
    if (!ctx.isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    if (typeof projectId !== 'string' || !PROJECT_ID.test(projectId) || !ctx.projects) throw new Error('Invalid project id');
    const grants = new Map<string, { projectId: string; path: string }>();
    const versions: import('../shared/ipc').ProjectVersion[] = [];
    const add = async (file: string, source: 'recovery' | 'file' | 'macos', at?: number) => {
      const id = randomUUID();
      grants.set(id, { projectId, path: file });
      versions.push({ id, source, at: at ?? (await (await import('node:fs/promises')).stat(file)).mtimeMs });
    };
    for (const file of await ctx.projects.backups(projectId)) await add(file, 'file');
    if (ctx.recoveryProjects) {
      const current = await ctx.recoveryProjects.destination(projectId);
      if (current) await add(current, 'recovery');
      for (const file of await ctx.recoveryProjects.backups(projectId)) await add(file, 'recovery');
    }
    const destination = await ctx.projects.destination(projectId);
    if (destination) {
      try {
        const { listNativeVersions } = await import('./native-versions');
        for (const version of await listNativeVersions(destination)) await add(version.path, 'macos', version.at);
      } catch (error) { console.warn('macOS version history is unavailable.', error); }
    }
    versionGrants.set(event.sender, grants);
    return versions.sort((a, b) => b.at - a.at);
  });
  ipcMain.handle(IPC.projectVersionOpen, async (event, request: unknown): Promise<ProjectOpenResult> => {
    if (!ctx.isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    if (!isRecord(request) || typeof request.projectId !== 'string' || typeof request.versionId !== 'string' || !ctx.projects) throw new Error('Invalid version request');
    const grant = versionGrants.get(event.sender)?.get(request.versionId);
    if (!grant || grant.projectId !== request.projectId) throw new Error('This version is no longer available. Reopen Version History.');
    return openProjectForWindow({ projects: ctx.projects }, event.sender, grant.path, true);
  });
  ipcMain.handle(IPC.projectSaveBegin, async (event, request: unknown): Promise<ProjectSaveBeginResult> => {
    if (!ctx.isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    if (!ctx.projects || !isRecord(request) || typeof request.projectId !== 'string' || !PROJECT_ID.test(request.projectId)
      || (request.background !== undefined && typeof request.background !== 'boolean') || request.background === true && request.saveAs === true
      || (request.saveAs !== undefined && typeof request.saveAs !== 'boolean') || typeof request.documentBytes !== 'number'
      || !Number.isSafeInteger(request.documentBytes) || request.documentBytes < 2 || request.documentBytes > 0xffffffff) throw new Error('Invalid project save request');
    validateSaveMedia(request.media);
    const name = sanitizeSaveName(request.name);
    if (!name) throw new Error('Invalid project save name');
    if (pending.has(event.sender) || uploads.has(event.sender) || projectSaves.has(event.sender)) throw new Error('A save is already in progress.');
    const window = BrowserWindow.fromWebContents(event.sender);
    if (!window || window.isDestroyed()) throw new Error('Save window is unavailable');
    const marker = claimPending(event.sender); watchOwner(event.sender);
    let rendererGone = false;
    const gone = () => { rendererGone = true; };
    const checkRenderer = () => { if (rendererGone || isRendererUnavailable(event.sender)) throw new Error('Save cancelled'); };
    const stopWatching = watchRendererDocument(event.sender, gone);
    let id: string | undefined;
    try {
      checkRenderer();
      const known = await ctx.projects.destination(request.projectId);
      checkRenderer();
      let selectedPath: string | undefined;
      const recovery = request.background === true && !known;
      const target = recovery ? ctx.recoveryProjects : ctx.projects;
      if (!target) throw new Error('Background recovery is unavailable');
      if (recovery) {
        if (!await target.destination(request.projectId)) selectedPath = await target.recoveryPath(request.projectId);
      } else if (!known || request.saveAs) {
        const options = { defaultPath: known || name, filters: saveFiltersForName(name) };
        const result = ctx.dialogs?.showSave ? await ctx.dialogs.showSave(window, options) : await dialog.showSaveDialog(window, options);
        checkRenderer();
        if (result.canceled || !result.filePath) { releasePending(event.sender, marker); return { ok: false, cancelled: true }; }
        selectedPath = result.filePath;
      }
      if (window.isDestroyed() || event.sender.isDestroyed()) throw new Error('Save window is unavailable');
      id = randomUUID();
      const timer = setTimeout(() => { void discardProjectSave(event.sender, id).catch(() => undefined); }, 120_000); timer.unref();
      const ready = target.beginIncremental(request.projectId, request.media, request.documentBytes, selectedPath);
      projectSaves.set(event.sender, { id, ready, timer, finishing: false, marker, recovery });
      const save = await ready;
      checkRenderer();
      if (projectSaves.get(event.sender)?.id !== id || projectSaves.get(event.sender)?.disposing) throw new Error('Save cancelled');
      timer.refresh();
      return { ok: true, token: id, required: save.required };
    } catch (error: any) {
      if (id) await discardProjectSave(event.sender, id).catch(() => undefined);
      releasePending(event.sender, marker);
      return { ok: false, cancelled: false, error: error.message || 'Could not start saving.' };
    } finally { stopWatching(); }
  });
  ipcMain.handle(IPC.projectSaveChunk, async (event, request: unknown) => {
    if (!ctx.isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    const save = projectSaves.get(event.sender);
    if (!save || save.finishing || save.disposing || !isRecord(request) || request.token !== save.id
      || (request.assetId !== null && typeof request.assetId !== 'string') || !(request.data instanceof Uint8Array)
      || request.data.length < 1 || request.data.length > FILE_CHUNK_BYTES) throw new Error('Invalid project save chunk');
    save.timer.refresh();
    await (await save.ready).write(request.assetId, request.data);
    if (projectSaves.get(event.sender) === save && !save.disposing) save.timer.refresh();
  });
  ipcMain.handle(IPC.projectSaveFinish, async (event, token: unknown): Promise<FileSaveResult> => {
    if (!ctx.isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    const save = projectSaves.get(event.sender);
    if (!save || save.finishing || save.disposing || save.id !== token) throw new Error('Unknown project save token');
    save.finishing = true; clearTimeout(save.timer);
    try { return { ok: true, path: await (await save.ready).finish(), ...(save.recovery ? { recovery: true } : {}) }; }
    catch (error: any) { return { ok: false, cancelled: false, error: error.message || 'Could not save project.' }; }
    finally { await discardProjectSave(event.sender, save.id); }
  });
  ipcMain.handle(IPC.projectSaveAbort, async (event, token: unknown) => {
    if (!ctx.isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    if (typeof token === 'string') await discardProjectSave(event.sender, token);
  });
  ipcMain.handle(IPC.exportChoose, async (event, request: unknown) => {
    if (!ctx.isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    if (!isRecord(request)) throw new Error('Invalid export destination');
    const name = sanitizeSaveName(request.name);
    if (!name || (request.directory !== undefined && typeof request.directory !== 'boolean')) throw new Error('Invalid export destination');
    if (pending.has(event.sender) || destinations.has(event.sender)) throw new Error('An export is already in progress.');
    const window = BrowserWindow.fromWebContents(event.sender);
    if (!window || window.isDestroyed()) throw new Error('Save window is unavailable');
    const marker = claimPending(event.sender);
    watchOwner(event.sender);
    let rendererGone = false;
    const gone = () => { rendererGone = true; };
    const isCurrent = () => !rendererGone && !isRendererUnavailable(event.sender);
    const stopWatching = watchRendererDocument(event.sender, gone);
    try {
      if (!isCurrent()) return null;
      let selectedPath: string | undefined;
      if (request.directory) {
        const result = await dialog.showOpenDialog(window, { title: 'Export PNG Sequence', properties: ['openDirectory', 'createDirectory'] });
        if (!result.canceled) selectedPath = result.filePaths[0];
      } else {
        const options = { defaultPath: name, filters: saveFiltersForName(name) };
        selectedPath = await agentExportDestination(event.sender, name);
        if (!isCurrent()) return null;
        if (!selectedPath) {
          const result = ctx.dialogs?.showSave ? await ctx.dialogs.showSave(window, options) : await dialog.showSaveDialog(window, options);
          if (!result.canceled) selectedPath = result.filePath;
        }
      }
      if (!selectedPath || window.isDestroyed() || !isCurrent()) return null;
      const token = randomUUID();
      destinations.set(event.sender, { token, path: selectedPath, directory: !!request.directory });
      return token;
    } finally {
      stopWatching();
      releasePending(event.sender, marker);
    }
  });
  ipcMain.handle(IPC.exportRelease, (event, token: unknown) => {
    if (!ctx.isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    if (destinations.get(event.sender)?.token === token) destinations.delete(event.sender);
  });
  ipcMain.handle(IPC.fileSaveUpload, async (event, size: unknown) => {
    if (!ctx.isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    if (typeof size !== 'number' || !Number.isSafeInteger(size) || size < 1) throw new Error('Invalid save size');
    if (uploads.has(event.sender) || pending.has(event.sender)) throw new Error('A save is already in progress.');
    if (isRendererUnavailable(event.sender)) throw new Error('Save cancelled');
    const id = randomUUID();
    const timer = setTimeout(() => { void discardUpload(event.sender, id).catch(() => undefined); }, 120_000); timer.unref();
    const ready = FileUpload.create(size);
    uploads.set(event.sender, { id, ready, timer }); watchOwner(event.sender);
    try {
      await ready;
      if (isRendererUnavailable(event.sender) || uploads.get(event.sender)?.id !== id || uploads.get(event.sender)?.disposing) throw new Error('Save cancelled');
      return id;
    } catch (error) { await discardUpload(event.sender, id).catch(() => undefined); throw error; }
  });
  ipcMain.handle(IPC.fileSaveChunk, async (event, payload: unknown) => {
    if (!ctx.isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    const upload = uploads.get(event.sender);
    if (!upload || upload.disposing || !isRecord(payload) || payload.uploadId !== upload.id || !(payload.data instanceof Uint8Array)
      || payload.data.byteLength < 1 || payload.data.byteLength > FILE_CHUNK_BYTES) throw new Error('Invalid save chunk');
    upload.timer.refresh();
    await (await upload.ready).write(payload.data);
    if (uploads.get(event.sender) === upload && !upload.disposing) upload.timer.refresh();
  });
  ipcMain.handle(IPC.fileSaveAbort, async (event, id: unknown) => {
    if (!ctx.isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    if (typeof id === 'string') await discardUpload(event.sender, id);
  });
  ipcMain.handle(IPC.projectRead, async (event, request: unknown) => {
    if (!ctx.isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    if (!ctx.projects) throw new Error('Project storage is unavailable');
    const readers = projectReaderState(ctx.projects).readers;
    if (!isRecord(request) || typeof request.token !== 'string' || readers.get(request.token)?.owner !== event.sender) throw new Error('Unknown project read token');
    const reader = readers.get(request.token)!; reader.timer.refresh();
    const data = await ctx.projects!.read(request.token, request.offset as number, request.length as number);
    if (readers.get(request.token) === reader) reader.timer.refresh();
    return data;
  });
  ipcMain.handle(IPC.projectReadClose, async (event, token: unknown) => {
    if (!ctx.isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    if (!ctx.projects) throw new Error('Project storage is unavailable');
    const readers = projectReaderState(ctx.projects).readers;
    if (typeof token !== 'string' || readers.get(token)?.owner !== event.sender) throw new Error('Unknown project read token');
    await closeProjectReader(ctx.projects, token, true);
  });
  ipcMain.handle(IPC.fileSave, async (event, payload: unknown): Promise<FileSaveResult> => {
    if (!ctx.isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    if (!isRecord(payload)) throw new IpcValidationError(IPC.fileSave, 'expected an object');

    const name = sanitizeSaveName(payload['name']);
    if (name === null) throw new IpcValidationError(IPC.fileSave, 'invalid name');
    const projectId = payload['projectId'];
    if (projectId !== undefined && (typeof projectId !== 'string' || !PROJECT_ID.test(projectId))) {
      throw new IpcValidationError(IPC.fileSave, 'invalid project id');
    }
    if (payload['saveAs'] !== undefined && typeof payload['saveAs'] !== 'boolean') {
      throw new IpcValidationError(IPC.fileSave, 'invalid saveAs');
    }

    const destination = payload.destinationToken === undefined ? undefined : destinations.get(event.sender);
    if (payload.destinationToken !== undefined && (!destination || destination.token !== payload.destinationToken || projectId !== undefined)) {
      throw new Error('Unknown export destination');
    }
    if (destination?.directory && (name !== payload.name || path.extname(name).toLowerCase() !== '.png')) throw new Error('Invalid export frame name');

    let data = payload['data'];
    let stream: AsyncIterable<Uint8Array> | undefined;
    let publishUpload: ((destination: string) => Promise<void>) | undefined;
    if (payload.uploadId !== undefined) {
      const upload = uploads.get(event.sender);
      if (!upload || upload.disposing || payload.uploadId !== upload.id) throw new Error('The save upload is incomplete.');
      const staged = await upload.ready;
      // A busy response must leave the upload retryable and its idle cleanup
      // running. Claiming it first consumes the spool and cancels that timer.
      if (pending.has(event.sender)) return { ok: false, cancelled: false, error: 'A save is already in progress.' };
      if (projectId && ctx.projects) stream = staged.claim();
      else publishUpload = staged.claimForExport();
      clearTimeout(upload.timer);
    }
    if (!stream && !publishUpload && !(data instanceof Uint8Array)) {
      throw new IpcValidationError(IPC.fileSave, 'data must be Uint8Array');
    }
    if (!stream && !publishUpload && !isBytes(data, LIMITS.fileSaveBytes)) {
      return { ok: false, cancelled: false, error: 'too large; use streaming export' };
    }

    if (pending.has(event.sender)) return { ok: false, cancelled: false, error: 'A save is already in progress.' };
    const marker = claimPending(event.sender);
    watchOwner(event.sender);
    let rendererGone = false;
    const gone = () => { rendererGone = true; };
    const checkRenderer = () => { if (rendererGone || isRendererUnavailable(event.sender)) throw new Error('Save cancelled'); };
    const stopWatching = watchRendererDocument(event.sender, gone);
    let saveFailed = false;
    try {
      checkRenderer();
      const window = BrowserWindow.fromWebContents(event.sender);
      if (!window || window.isDestroyed()) throw new Error('Save window is unavailable');
      const known = projectId && ctx.projects ? await ctx.projects.destination(projectId) : undefined;
      checkRenderer();
      let selectedPath = destination ? (destination.directory ? path.join(destination.path, name) : destination.path) : undefined;
      if (!destination && (!known || payload['saveAs'])) {
        const options: SaveDialogOptions = { defaultPath: known || name, filters: saveFiltersForName(name) };
        const result = ctx.dialogs?.showSave
          ? await ctx.dialogs.showSave(window, options)
          : await dialog.showSaveDialog(window, options);
        checkRenderer();
        if (result.canceled || !result.filePath) return { ok: false, cancelled: true };
        selectedPath = result.filePath;
      }
      // Once the writer starts, its durable outcome must still reach the
      // caller. Only abandoned destination selection is cancelled here.
      checkRenderer();
      if (projectId && ctx.projects) {
        return { ok: true, path: await ctx.projects.save(projectId, stream || data as Uint8Array, selectedPath) };
      }
      if (publishUpload) await publishUpload(selectedPath!);
      else await atomicWrite(selectedPath!, data as Uint8Array);
      if (destination && !destination.directory) rememberExport(event.sender.id, selectedPath!);
      return { ok: true, path: selectedPath! };
    } catch (error: any) {
      saveFailed = true;
      return { ok: false, cancelled: false, error: error.message || 'Save failed. Check disk space and folder permissions.' };
    } finally {
      stopWatching();
      releasePending(event.sender, marker);
      if (typeof payload.uploadId === 'string') {
        const cleanup = discardUpload(event.sender, payload.uploadId);
        if (saveFailed) await cleanup.catch(() => undefined);
        else await cleanup;
      }
    }
  });
  ipcMain.handle(IPC.projectOpen, async (event): Promise<ProjectOpenResult> => {
    if (!ctx.isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    const window = BrowserWindow.fromWebContents(event.sender);
    if (!window || window.isDestroyed() || !ctx.projects || isRendererUnavailable(event.sender)) throw new Error('Project window is unavailable');
    let documentGone = false;
    const stopWatching = watchRendererDocument(event.sender, () => { documentGone = true; });
    try {
      const result = await dialog.showOpenDialog(window, {
        title: 'Open Project', properties: ['openFile'],
        filters: [{ name: 'Powermove Project', extensions: ['pmv', 'json'] }]
      });
      if (documentGone || isRendererUnavailable(event.sender)) throw new Error('Project window is unavailable');
      if (result.canceled || !result.filePaths[0]) return { ok: false, cancelled: true };
      return await openProjectForWindow({ projects: ctx.projects }, event.sender, result.filePaths[0]);
    } catch (error: any) { return { ok: false, cancelled: false, error: error.message || 'Could not open project.' }; }
    finally { stopWatching(); }
  });
  ipcMain.handle(IPC.projectConfirmClose, async (event, name: unknown): Promise<CloseDecision> => {
    if (!ctx.isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    if (typeof name !== 'string' || name.length > 1000) throw new IpcValidationError(IPC.projectConfirmClose, 'invalid name');
    const window = BrowserWindow.fromWebContents(event.sender);
    if (!window || window.isDestroyed()) return 'cancel';
    const result = await dialog.showMessageBox(window, {
      type: 'question', message: `Save changes to “${name}” before closing?`,
      detail: 'Your project file has not been updated. Local recovery is kept separately.',
      buttons: ['Save', 'Cancel', 'Don’t Save'], defaultId: 0, cancelId: 1, noLink: true
    });
    return result.response === 0 ? 'save' : result.response === 2 ? 'discard' : 'cancel';
  });
}

export { MAX_SAVE_NAME_CHARS };
